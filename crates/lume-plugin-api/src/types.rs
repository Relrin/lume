//! What a plugin's code sends with each call, and what it is given back.
//!
//! Each type here is read from what the code sent, strictly: a field Lume does not
//! know is an error, so a mistake shows at once rather than as something that is
//! quietly ignored.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Nothing: what a call that takes no arguments is sent.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Nothing {}

/// What a status item shows. Each field left out keeps what the item showed before;
/// the manifest says what kind of item it is.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct StatusValue {
    /// The words of a text or badge item, or the amount written out beside a meter or
    /// a sparkline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// What the item stands for, read out and shown beside a meter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// How full a meter is, from 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// Which part of a segmented item is the chosen one, by its value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<String>,
    /// A sparkline's values, from the oldest to the newest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    /// One of the theme's colours, by its token's name, as `color.tab.purple`: what
    /// the item draws is drawn in it, in place of the tone's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// What the pointer is told when it rests on the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    /// The picture beside it: one of the plugin's own, by the name its manifest gives
    /// it, or one of the application's, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The rows of the card shown when the pointer rests on the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<Vec<CardRow>>,
    /// Whether the item is out of sight for now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
}

/// What a status item's colour says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

/// One row of a status item's card: a name and what it is, perhaps with a line through
/// its latest values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CardRow {
    pub label: String,
    pub value: String,
    /// The latest values, from the oldest to the newest, drawn as a small line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
}

/// One row of a list panel. Rows hold rows of their own, which open beneath them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PanelRow {
    /// Names the row among its siblings, for the actions run on it.
    pub id: String,
    pub label: String,
    /// A few words after the label, in a quieter colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// One of the plugin's pictures, by the name its manifest gives it, or one of the
    /// application's, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// A count or a word at the row's end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<PanelRow>>,
    /// Whether the row has rows beneath it that are given only when the user opens it:
    /// `lume.panels.onExpand` hears of that, and `lume.panels.setChildren` gives them.
    /// A tree too large to build whole is shown this way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lazy: Option<bool>,
    /// Whether its rows are open beneath it at first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    /// What can be done with the row: each runs one of the plugin's commands, given
    /// `{ panel, row, path }` (the panel's name, the row's id and the ids from the top
    /// down to it) and the action's own arguments. The first is what choosing the row
    /// does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<RowAction>>,
}

/// Something that can be done with a row of a list panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RowAction {
    /// One of the plugin's commands, by the name its manifest gives it.
    pub command: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PanelsSet {
    /// The panel, by the name the manifest gives it.
    pub id: String,
    pub rows: Vec<PanelRow>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PanelId {
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PanelsSetChildren {
    /// The panel, by the name the manifest gives it.
    pub id: String,
    /// The row, by the ids of the rows from the top down to it.
    pub path: Vec<String>,
    pub rows: Vec<PanelRow>,
}

/// A row of a list panel the user opened whose rows are given when it is opened.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PanelExpand {
    /// The panel, by the name the manifest gives it.
    pub panel: String,
    /// The row's id.
    pub row: String,
    /// The ids of the rows from the top down to it.
    pub path: Vec<String>,
}

/// What the machine is doing, as one reading has it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetricsSample {
    /// How busy the processor is, from 0 to 1, over all its cores.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<f64>,
    /// How much memory is used, in bytes, of how much there is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryUsage>,
    /// What the graphics card is doing, where it can be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<GpuReading>,
    /// How fast the network goes, in bytes a second, over the hardware interfaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkRates>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryUsage {
    pub used: u64,
    pub total: u64,
}

/// What the graphics card is doing: `unavailable` where there is no way to read it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GpuReading {
    /// `read` or `unavailable`.
    pub state: String,
    /// How busy its busiest engine is, from 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy: Option<f64>,
    /// How much of its own memory is used, in bytes; the whole is not known for a card
    /// that shares the system's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<GpuMemoryUsage>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GpuMemoryUsage {
    pub used: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkRates {
    pub received: f64,
    pub sent: f64,
}

/// What a reading of the machine can be asked for.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum MetricKind {
    Cpu,
    Memory,
    Gpu,
    Network,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MetricsSubscribe {
    pub kinds: Vec<MetricKind>,
    /// The plugin's status items, by name, the readings are for: the machine is read
    /// only while one of them is on screen. Left empty, it is read all the while.
    #[serde(default)]
    pub visible: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MetricsUnsubscribe {
    pub subscription: u64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusSet {
    /// The status item, by the name the manifest gives it.
    pub id: String,
    pub value: StatusValue,
    /// The pane whose header shows it, for an item in a pane's header.
    #[serde(default)]
    pub pane: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusClear {
    pub id: String,
    #[serde(default)]
    pub pane: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandExecute {
    /// The command's id: one of the application's, or one of the plugin's own by its
    /// name.
    pub id: String,
    #[serde(default)]
    pub args: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KvKey {
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KvSet {
    pub key: String,
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SettingsGet {
    /// One setting, by its key; all of the plugin's if left out.
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextSet {
    /// The name of a condition the plugin's keys and menus can be shown under, as
    /// `plugin:<id>.<key>`.
    pub key: String,
    pub value: bool,
}

/// What the plugin can hear about.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Topic {
    /// What sessions report: titles, directories, commands that start and end.
    Session,
    /// The workspace in use: another one, or its root folder changed.
    Workspace,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Subscribe {
    pub topic: Topic,
}

/// Something a session reported, as a plugin hears it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionReport {
    /// The session.
    pub session: String,
    /// The pane that shows it, if one does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    /// The kind of report: `title`, `cwd`, `place`, `bell`, `command_started`,
    /// `command_finished`, `program_status`, `foreground`, `activity`,
    /// `disconnected`, `state` or `exit`.
    pub kind: String,
    /// What was reported, as the session reported it. A session on another machine, or
    /// one that a server runs, makes no `cwd` report, and its others give no folder and
    /// no number of a process. Where its machine is one that plugins can run programs
    /// on, it makes a `place` report instead, whose details are a [`Place`]: when where
    /// it is changes, and with each `state` report.
    pub details: Value,
}

/// How bytes are written where a call sends or gets back text: as text, or as base64
/// for what is not text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    #[default]
    Utf8,
    Base64,
}

/// A file or folder, as an absolute path or one that begins with `${workspace}`,
/// `${home}` or `${plugin-data}`.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FsPath {
    pub path: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FsReadOptions {
    #[serde(default)]
    pub encoding: Option<Encoding>,
    /// The most that is read; a larger file is refused. 16 MiB if left out.
    #[serde(default)]
    pub max_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FsRead {
    pub path: String,
    #[serde(default)]
    pub options: FsReadOptions,
}

/// How a file is written: in place of what it held, after it, or only where there is
/// no such file yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum WriteMode {
    #[default]
    Replace,
    Append,
    Create,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FsWriteOptions {
    #[serde(default)]
    pub encoding: Option<Encoding>,
    #[serde(default)]
    pub mode: Option<WriteMode>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FsWrite {
    pub path: String,
    pub data: String,
    #[serde(default)]
    pub options: FsWriteOptions,
}

/// What a path is: a file, a folder, or something else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
    Other,
}

/// What is known of a file or folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FsStat {
    pub kind: EntryKind,
    /// Its size in bytes.
    pub size: u64,
    /// When it last changed, in milliseconds since 1970, where that is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<u64>,
    pub readonly: bool,
}

/// One entry of a folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FsEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FsWatch {
    pub path: String,
    /// Whether what is in its folders is watched too.
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FsUnwatch {
    pub watch: u64,
}

/// What happened to a path that is watched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FsChange {
    Created,
    Changed,
    Removed,
    Other,
}

/// Something that happened to a file or folder that is watched.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FsEvent {
    pub path: String,
    pub kind: FsChange,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamReadOptions {
    /// The most bytes one read gives; 64 KiB if left out, 1 MiB at most.
    #[serde(default)]
    pub max: Option<u32>,
    #[serde(default)]
    pub encoding: Option<Encoding>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamRead {
    pub stream: u64,
    #[serde(default)]
    pub options: StreamReadOptions,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamWrite {
    pub stream: u64,
    pub data: String,
    #[serde(default)]
    pub encoding: Option<Encoding>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamId {
    pub stream: u64,
}

/// How a program is run.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProcessOptions {
    /// The folder it starts in: the plugin's own data folder if left out. On another
    /// machine it is an absolute path of that machine's, and the user's home folder if
    /// left out.
    #[serde(default)]
    pub cwd: Option<String>,
    /// The machine it runs on, by its id, as a session's `place` gives it; this one if
    /// left out. Another machine needs the permission `process.run-elsewhere` in place of
    /// `process.run`, and the user's word for that machine.
    #[serde(default)]
    pub machine: Option<String>,
    /// Variables added to its environment.
    #[serde(default)]
    pub env: Option<std::collections::BTreeMap<String, String>>,
    /// What is written to its input before it is closed.
    #[serde(default)]
    pub stdin: Option<String>,
    /// How long it may run before it is stopped; 30 s if left out, 10 minutes at most.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// How its input is given and its output given back.
    #[serde(default)]
    pub encoding: Option<Encoding>,
    /// The most output kept of each of its two streams; 4 MiB if left out.
    #[serde(default)]
    pub max_output: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcessRun {
    /// The program, by its name alone, as `git`.
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub options: ProcessOptions,
}

/// How a program that was run ended.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResult {
    /// Its exit code; null if it was stopped.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// Whether it ran out of time and was stopped.
    pub timed_out: bool,
    /// Whether some of its output was left out for being too long.
    pub truncated: bool,
}

/// A program that is started and left running.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpawnOptions {
    #[serde(default)]
    pub cwd: Option<String>,
    /// The machine it runs on, as for `process.run`. A program started on another
    /// machine reads nothing: its input is closed.
    #[serde(default)]
    pub machine: Option<String>,
    #[serde(default)]
    pub env: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcessSpawn {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub options: SpawnOptions,
}

/// A program that runs, and the streams of its input and output.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Spawned {
    /// The program, for `process.wait` and `process.kill`.
    pub process: u64,
    /// The system's number of the process; 0 on another machine.
    pub pid: u32,
    pub stdin: u64,
    pub stdout: u64,
    pub stderr: u64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcessId {
    pub process: u64,
}

/// How a program that was started ended.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProcessExit {
    /// Its exit code; null if it was stopped.
    pub code: Option<i32>,
}

/// What a request to a server sends.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FetchInit {
    /// `GET` if left out.
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub headers: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    pub body: Option<String>,
    /// How the body sent and the body given back are written.
    #[serde(default)]
    pub encoding: Option<Encoding>,
    /// 30 s if left out, 5 minutes at most.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// The largest answer taken; 16 MiB if left out.
    #[serde(default)]
    pub max_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NetFetch {
    pub url: String,
    #[serde(default)]
    pub init: FetchInit,
}

/// A server's answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FetchResult {
    pub status: u16,
    /// Its headers, by their names in lower case; repeated ones joined with `, `.
    pub headers: std::collections::BTreeMap<String, String>,
    /// Where the answer came from, after any redirects.
    pub url: String,
    pub body: String,
}

/// A server's answer whose body is read from a stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FetchStreamResult {
    pub status: u16,
    pub headers: std::collections::BTreeMap<String, String>,
    pub url: String,
    pub stream: u64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretKey {
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretSet {
    pub key: String,
    pub secret: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipboardWrite {
    pub text: String,
}

/// A terminal pane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PaneInfo {
    pub pane: String,
    /// The session it shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Whether it has the keyboard in its window.
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaneRef {
    pub pane: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaneWrite {
    pub pane: String,
    /// What is typed, as it is: a line ends with `\r` to be run.
    pub text: String,
}

/// What the programs of a pane use: the one it started and everything it started.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PaneStats {
    pub processes: u32,
    /// Memory in use, in bytes.
    pub memory: u64,
    /// How much of the processor they used since the pane was last asked about, from 0
    /// to 1 over all its cores; null the first time.
    pub cpu: Option<f64>,
}

/// Where a new terminal opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SpawnPlace {
    /// A new tab of the window in front.
    #[default]
    Tab,
    /// Beside the pane that has the keyboard.
    Right,
    /// Below it.
    Down,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionSpawnOptions {
    /// The folder it starts in.
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub place: Option<SpawnPlace>,
    /// A program to run in it in place of the profile's shell, by its name alone, as
    /// `claude`. The plugin needs the `process.run` permission over it as well.
    #[serde(default)]
    pub program: Option<String>,
    /// The program's arguments. Only with `program`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Variables added to the program's environment. Only with `program`.
    #[serde(default)]
    pub env: Option<std::collections::BTreeMap<String, String>>,
    /// The pane it opens beside, for `right` and `down`: the pane that has the keyboard
    /// if left out.
    #[serde(default)]
    pub pane: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionsSpawn {
    /// One of the user's profiles, by its id; the default profile if left out.
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub options: SessionSpawnOptions,
}

/// A session: a program running in a terminal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionEntry {
    pub session: String,
    /// The pane that shows it, if one does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    pub program: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The folder it is in, as a path of this machine. Left out where the folder is not
    /// known or not there, and for a session on another machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Where it is, if that is another machine that plugins can run programs on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
    /// Whether its program has ended.
    pub exited: bool,
}

/// Where a session is that is not on this machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Place {
    /// The machine's id, for `process.run` and `machines.allow`. It stays the same as
    /// long as the machine is reached the same way.
    pub machine: String,
    /// What to call the machine.
    pub label: String,
    pub kind: MachineKind,
    /// The folder the session is in, as a path of that machine, where it is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

/// How a machine is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MachineKind {
    /// A server that runs the session.
    Rex,
    /// The SSH client.
    Ssh,
    /// A Linux distribution of this Windows machine.
    Wsl,
}

/// A machine that sessions are on, other than this one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MachineEntry {
    pub machine: String,
    pub label: String,
    pub kind: MachineKind,
    /// Whether the plugin may run its programs there.
    pub allowed: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MachinesAllow {
    /// The machine, by its id.
    pub machine: String,
    /// Why the plugin asks, in a sentence the user is shown as the plugin's words.
    #[serde(default)]
    pub reason: Option<String>,
}

/// The workspace in use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceInfo {
    pub name: String,
    /// Its root folder, which the user sets; null where none is set.
    pub root: Option<String>,
    pub windows: u32,
    pub tabs: u32,
    pub panes: u32,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayoutApply {
    /// What to change, as the layout's intents are written.
    pub intent: Value,
}

/// How a notice looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NoticeLevel {
    #[default]
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiToast {
    /// At most 300 characters.
    pub message: String,
    #[serde(default)]
    pub level: Option<NoticeLevel>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfirmOptions {
    /// The label of the button that goes ahead: `OK` if left out.
    #[serde(default)]
    pub confirm: Option<String>,
    /// Whether going ahead cannot be taken back, which the button shows.
    #[serde(default)]
    pub danger: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiConfirm {
    /// At most 80 characters.
    pub title: String,
    /// At most 1000 characters.
    pub message: String,
    #[serde(default)]
    pub options: ConfirmOptions,
}

/// One of the choices of a pick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PickItem {
    pub id: String,
    /// At most 120 characters.
    pub label: String,
    /// At most 200 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PickOptions {
    #[serde(default)]
    pub title: Option<String>,
    /// What the field says before anything is typed.
    #[serde(default)]
    pub placeholder: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiPick {
    /// At most 500.
    pub items: Vec<PickItem>,
    #[serde(default)]
    pub options: PickOptions,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PermissionsRequest {
    /// A kind the manifest lists under `[permissions.optional]`, as `clipboard.read`.
    pub permission: String,
    /// What it should cover, for a kind that has a scope: paths, programs, servers or
    /// plugins, written as a manifest writes them.
    #[serde(default)]
    pub scope: Option<Vec<String>>,
    /// Why the plugin asks, shown to the user under its name. At most 300 characters.
    #[serde(default)]
    pub reason: Option<String>,
}

/// Whether the user is asked before each use of a permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PromptWhen {
    Once,
    Each,
}

/// A permission the plugin holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Grant {
    pub permission: String,
    /// What it covers, for a kind that has a scope.
    pub scope: Vec<String>,
    pub prompt: PromptWhen,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewsPost {
    /// An open view's instance, or a view's name for every open one of it.
    pub to: String,
    /// The event's name, as the view listens for it.
    pub event: String,
    #[serde(default)]
    pub payload: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewsInstances {
    /// One view's name; all of the plugin's views when left out.
    #[serde(default)]
    pub view: Option<String>,
}

/// One open view of the plugin's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewInstance {
    /// What tells this open view from every other: calls from it carry it, and
    /// `views.post` reaches it by it.
    pub instance: String,
    /// The view's name, as the manifest gives it.
    pub view: String,
    /// Where it is shown: `pane`, `panel`, or `settings` for the plugin's page in the
    /// settings window.
    pub place: String,
    /// The pane it is shown in, for a view in a pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
}

/// Where a view opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ViewPlace {
    /// A new tab of the window in front.
    #[default]
    Tab,
    /// Beside the pane that has the keyboard.
    Right,
    /// Below it.
    Down,
    /// Floating over the window's tabs.
    Float,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenViewOptions {
    /// What the view starts with, as its `state`: JSON of 64 KiB at most.
    #[serde(default)]
    pub state: Option<Value>,
    #[serde(default)]
    pub place: Option<ViewPlace>,
    /// What its pane is called; the view's title if left out.
    #[serde(default)]
    pub title: Option<String>,
    /// Shows a pane that has this view open already, if there is one, instead of
    /// opening another. Such a pane keeps the state it has.
    #[serde(default)]
    pub reuse: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiOpenView {
    /// One of the plugin's pane views, by the name its manifest gives it.
    pub view: String,
    #[serde(default)]
    pub options: OpenViewOptions,
}

/// The pane a view was opened in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpenedView {
    pub pane: String,
    /// Whether the pane was there already and was only shown.
    pub reused: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamGrant {
    /// The open view that may use the stream, by its instance.
    pub instance: String,
    pub stream: u64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamRevoke {
    /// What `streams.grantToView` gave.
    pub grant: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServicesCall {
    /// The plugin that offers the service, by the id its manifest gives it.
    pub plugin: String,
    /// The service, by the name its manifest gives it.
    pub name: String,
    #[serde(default)]
    pub args: Option<Value>,
}

/// Why a call failed.
///
/// The codes are the application's own, and few. A plugin's code names its own kinds
/// of failure itself (`kind`) and says what it likes of each (`data`); the application
/// carries both to whoever called, a view or another plugin, without knowing any of
/// them. So a plugin gains a kind of failure without anything here changing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    /// The kind of failure, as the plugin whose code failed names it. The caller reads
    /// it where it would read the code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// What the plugin's code says of the failure, as JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// The longest a plugin's own kind of failure may be named.
pub const ERROR_KIND_MAX: usize = 64;

/// The most a plugin's code may say of a failure, as JSON.
pub const ERROR_DATA_MAX: usize = 16 * 1024;

/// Whether `kind` can name a plugin's own kind of failure: lower-case letters, digits,
/// `_`, `-` and `.`, beginning with a letter.
pub fn error_kind_ok(kind: &str) -> bool {
    kind.len() <= ERROR_KIND_MAX
        && kind.starts_with(|c: char| c.is_ascii_lowercase())
        && kind
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.'))
}

/// The kinds of failure a call can have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The plugin has no permission for it.
    Denied,
    /// What was sent is not what the call takes.
    Invalid,
    /// What was asked for is not there.
    NotFound,
    /// The plugin is making more calls than it may at once.
    Busy,
    /// It could not be done.
    Failed,
    /// The plugin was stopped before it was done.
    Stopped,
    /// The command was not handled by the plugin's code.
    Unhandled,
    /// What was asked for, or what came back, is larger than the call allows.
    TooLarge,
    /// What the call needs is not there on this machine, as a keychain.
    Unavailable,
    /// It took longer than it may.
    TimedOut,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            kind: None,
            data: None,
        }
    }

    /// The failure as a plugin's code reported it: as JSON with a code that may be the
    /// plugin's own. A code that is none of the application's becomes the failure's
    /// kind, under the code `failed`; what cannot be a kind, and data that is too
    /// large, are left out, and the failure is still a failure.
    pub fn reported(error: &Value) -> Self {
        let message = match error.get("message") {
            Some(Value::String(message)) => message.clone(),
            _ => error.to_string(),
        };
        let named = error.get("code").and_then(Value::as_str);
        let code: Option<ErrorCode> =
            named.and_then(|named| serde_json::from_value(Value::from(named)).ok());
        let kind = match code {
            Some(_) => None,
            None => named
                .filter(|named| error_kind_ok(named))
                .map(str::to_owned),
        };
        let data = error
            .get("data")
            .filter(|data| !data.is_null())
            .filter(|data| data.to_string().len() <= ERROR_DATA_MAX)
            .cloned();
        Self {
            code: code.unwrap_or(ErrorCode::Failed),
            message,
            kind,
            data,
        }
    }

    /// What the caller reads as the failure's code: the plugin's own kind where it
    /// named one, the application's code otherwise.
    pub fn shown_code(&self) -> String {
        match &self.kind {
            Some(kind) => kind.clone(),
            None => serde_json::to_value(self.code)
                .ok()
                .and_then(|code| code.as_str().map(str::to_owned))
                .unwrap_or_else(|| "failed".to_owned()),
        }
    }

    pub fn denied(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Denied, message)
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Invalid, message)
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Failed, message)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}
