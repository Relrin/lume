//! What a client and a Rex server say to each other, as far as this client uses it.
//!
//! The shapes are those of API revision 19, as the server describes them. A field this
//! client does not use is left out here and ignored when a server sends it; a newer
//! server may send more. Everything read is cut to size before it is used.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use lume_proto::SessionEvent;

use crate::error::RexError;
use crate::events;
use crate::tuning::{
    BLOCKS_MAX, CLIENT_TEXT_MAX_BYTES, LAYERS_MAX, SPLIT_DEPTH_MAX, TEXT_MAX_BYTES, WINDOWS_MAX,
};

/// Cuts a string to at most `max` bytes, at a character's edge.
pub(crate) fn cut(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

/// A list that a server may write as `null` when it is empty.
fn or_empty<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

/// What this client says it is, for the server to show to people.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClientInfo {
    /// What sort of program it is: `app`, `cli`, `agent` or `other`.
    pub kind: String,
    /// What it runs on: `windows`, `linux`, `macos` and so on.
    pub platform: String,
    pub name: String,
    pub version: String,
    /// A name a person gave this client, if one did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl ClientInfo {
    /// A client that is an application called `name`, on the system this was built for.
    pub fn app(name: &str, version: &str) -> Self {
        let platform = match std::env::consts::OS {
            os @ ("windows" | "linux" | "macos" | "ios" | "freebsd") => os,
            _ => "other",
        };
        let mut info = Self {
            kind: "app".to_owned(),
            platform: platform.to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
            label: None,
        };
        info.cut();
        info
    }

    /// Keeps what is sent short: a hello has little room, and a server cuts or drops
    /// what is too long anyway.
    pub(crate) fn cut(&mut self) {
        for text in [
            &mut self.kind,
            &mut self.platform,
            &mut self.name,
            &mut self.version,
        ] {
            cut(text, CLIENT_TEXT_MAX_BYTES);
        }
        if let Some(label) = &mut self.label {
            cut(label, CLIENT_TEXT_MAX_BYTES);
        }
    }
}

/// The first message a client sends.
#[derive(Serialize)]
pub(crate) struct Hello<'a> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub api_revision: u32,
    pub api_min_revision: u32,
    pub client: &'a ClientInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconnect_token: Option<&'a str>,
}

/// Something a server gave this client to keep to itself. It is never written out.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Secret(pub(crate) String);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(…)")
    }
}

#[derive(Deserialize)]
pub(crate) struct WireWelcome {
    pub protocol: String,
    pub limits: WireLimits,
    #[serde(default)]
    pub client: Option<WireWelcomeClient>,
    #[serde(default)]
    pub api_revision: u32,
    #[serde(default)]
    pub api_server_revision: u32,
    #[serde(default)]
    pub api_min_revision: u32,
}

#[derive(Deserialize)]
pub(crate) struct WireLimits {
    pub max_message_bytes: u64,
}

#[derive(Deserialize)]
pub(crate) struct WireWelcomeClient {
    pub id: String,
    pub reconnect_token: String,
    #[serde(default)]
    pub resumed: bool,
}

/// The revisions of the control API a connection and its server speak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApiRevisions {
    /// The revision every message on the connection uses.
    pub used: u32,
    /// The newest the server speaks.
    pub server: u32,
    /// The oldest the server speaks.
    pub server_min: u32,
}

/// What a server answered a hello with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    /// The id the server knows this client by. It stays when the client reconnects.
    pub client_id: Option<String>,
    /// Whether the connection continued a client the server still had.
    pub resumed: bool,
    /// The largest message the server sends or accepts, in bytes.
    pub max_message_bytes: usize,
    pub revisions: ApiRevisions,
}

/// The version of Rex a server runs.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ServerVersion {
    pub rex_version: String,
    pub platform: String,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub api_revision: u32,
    #[serde(default)]
    pub api_min_revision: u32,
}

impl ServerVersion {
    pub(crate) fn cut(mut self) -> Self {
        cut(&mut self.rex_version, TEXT_MAX_BYTES);
        cut(&mut self.platform, TEXT_MAX_BYTES);
        if let Some(commit) = &mut self.commit {
            cut(commit, TEXT_MAX_BYTES);
        }
        self
    }
}

/// A session as a server lists it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub label: String,
}

impl SessionSummary {
    pub(crate) fn cut(mut self) -> Self {
        cut(&mut self.session_id, TEXT_MAX_BYTES);
        cut(&mut self.label, TEXT_MAX_BYTES);
        self
    }
}

/// What `session.attach_all` and `session.list` return.
#[derive(Deserialize)]
pub(crate) struct SessionList {
    #[serde(deserialize_with = "or_empty")]
    pub sessions: Vec<SessionSummary>,
}

/// What `session.attach` returns.
#[derive(Deserialize)]
pub(crate) struct Attached {
    pub view: View,
}

/// A session's windows, their layouts and their blocks, as they stood at one revision.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct View {
    /// Goes up by one with every change to what the view reports.
    pub revision: u64,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub active_window_id: Option<String>,
    #[serde(default, deserialize_with = "or_empty")]
    pub windows: Vec<ViewWindow>,
}

/// A window of a session: a tab.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ViewWindow {
    pub window_id: Option<String>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub focused_block_id: Option<String>,
    #[serde(default)]
    pub zoomed_block_id: Option<String>,
    #[serde(deserialize_with = "or_empty")]
    pub layers: Vec<ViewLayer>,
}

/// A layer of a window: the tiled one that holds its layout, or one that floats over it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ViewLayer {
    pub layer_id: Option<String>,
    /// `tiled` for the layout, anything else for a layer that floats.
    pub kind: String,
    /// Where the layer is in its window: x, y, width and height as fractions of it.
    pub bounds: [f64; 4],
    /// `minimized` while the layer is put away; left out otherwise.
    #[serde(default)]
    pub state: Option<String>,
    pub layout: Option<ViewNode>,
    #[serde(deserialize_with = "or_empty")]
    pub blocks: Vec<ViewBlock>,
}

/// A node of a layout: a block, or a split with a node on each side.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ViewNode {
    pub rect: [f64; 4],
    #[serde(default)]
    pub block_id: Option<String>,
    #[serde(default)]
    pub split: Option<Box<ViewSplit>>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ViewSplit {
    pub split_id: Option<String>,
    /// `horizontal` for side by side, `vertical` for one above the other.
    pub direction: String,
    /// The first side's share of the split.
    pub ratio: f64,
    pub before: ViewNode,
    pub after: ViewNode,
}

/// A block as a view names it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ViewBlock {
    pub block_id: Option<String>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub creator_name: String,
    #[serde(default)]
    pub flavor: String,
    #[serde(default)]
    pub owner: Option<String>,
}

/// The creator of the blocks that are terminals.
pub(crate) const TERMINAL_CREATOR: &str = "com.superlogical.terminal";

impl ViewBlock {
    /// Whether the block is a terminal.
    pub fn is_terminal(&self) -> bool {
        self.creator_name == TERMINAL_CREATOR
    }
}

fn cut_id(id: &mut Option<String>) {
    if let Some(id) = id {
        cut(id, TEXT_MAX_BYTES);
    }
}

fn finite(numbers: &[f64; 4], what: &str) -> Result<(), RexError> {
    if numbers.iter().all(|number| number.is_finite()) {
        Ok(())
    } else {
        Err(RexError::Protocol(format!("{what} is not made of numbers")))
    }
}

impl ViewNode {
    fn check(&mut self, depth: usize, blocks: &mut usize) -> Result<(), RexError> {
        if depth > SPLIT_DEPTH_MAX {
            return Err(RexError::Protocol(
                "a layout's splits nest too deep".to_owned(),
            ));
        }
        finite(&self.rect, "a layout node's rectangle")?;
        cut_id(&mut self.block_id);
        if self.block_id.is_some() {
            *blocks += 1;
        }
        if let Some(split) = &mut self.split {
            if !split.ratio.is_finite() {
                return Err(RexError::Protocol(
                    "a split's ratio is not a number".to_owned(),
                ));
            }
            cut_id(&mut split.split_id);
            cut(&mut split.direction, TEXT_MAX_BYTES);
            split.before.check(depth + 1, blocks)?;
            split.after.check(depth + 1, blocks)?;
        }
        Ok(())
    }
}

impl View {
    /// Makes a view safe to use: what is too long is cut, and a view with more in it
    /// than any server holds, or with numbers that are not numbers, is refused.
    pub(crate) fn check(mut self) -> Result<Self, RexError> {
        cut_id(&mut self.session_id);
        cut_id(&mut self.active_window_id);
        cut(&mut self.label, TEXT_MAX_BYTES);
        if self.windows.len() > WINDOWS_MAX {
            return Err(RexError::Protocol("a view has too many windows".to_owned()));
        }
        let mut blocks = 0usize;
        for window in &mut self.windows {
            cut_id(&mut window.window_id);
            cut_id(&mut window.focused_block_id);
            cut_id(&mut window.zoomed_block_id);
            cut(&mut window.label, TEXT_MAX_BYTES);
            if window.layers.len() > LAYERS_MAX {
                return Err(RexError::Protocol(
                    "a window has too many layers".to_owned(),
                ));
            }
            for layer in &mut window.layers {
                cut_id(&mut layer.layer_id);
                cut(&mut layer.kind, TEXT_MAX_BYTES);
                cut_id(&mut layer.state);
                finite(&layer.bounds, "a layer's bounds")?;
                if let Some(layout) = &mut layer.layout {
                    layout.check(0, &mut blocks)?;
                }
                blocks += layer.blocks.len();
                for block in &mut layer.blocks {
                    cut_id(&mut block.block_id);
                    cut_id(&mut block.owner);
                    cut(&mut block.label, TEXT_MAX_BYTES);
                    cut(&mut block.creator_name, TEXT_MAX_BYTES);
                    cut(&mut block.flavor, TEXT_MAX_BYTES);
                }
                if blocks > BLOCKS_MAX {
                    return Err(RexError::Protocol("a view has too many blocks".to_owned()));
                }
            }
        }
        Ok(self)
    }

    /// Every block of the view, in the order of its windows and layers.
    pub fn blocks(&self) -> impl Iterator<Item = &ViewBlock> {
        self.windows
            .iter()
            .flat_map(|window| &window.layers)
            .flat_map(|layer| &layer.blocks)
    }
}

/// What `com.superlogical.terminal.size` returns.
#[derive(Deserialize)]
pub(crate) struct TerminalSize {
    pub columns: u32,
    pub rows: u32,
}

/// A process as a terminal reports it.
#[derive(Deserialize)]
pub(crate) struct Process {
    pub pid: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct LastExit {
    pub exit_code: i64,
}

/// What `com.superlogical.terminal.process` returns.
#[derive(Deserialize)]
pub(crate) struct TerminalProcess {
    #[serde(default)]
    pub last_exit: Option<LastExit>,
    #[serde(default)]
    pub child: Option<Process>,
    #[serde(default)]
    pub foreground: Option<Process>,
}

/// What `com.superlogical.terminal.title` returns.
#[derive(Deserialize)]
pub(crate) struct TerminalTitle {
    #[serde(default)]
    pub title: String,
}

/// What `com.superlogical.terminal.modes` returns, as far as it is used.
#[derive(Deserialize)]
pub(crate) struct TerminalModes {
    /// `primary`, or `alternate` while a full-screen program has switched to it.
    #[serde(default)]
    pub screen: String,
}

/// What `data.connect` returns.
#[derive(Deserialize)]
pub(crate) struct Ticketed {
    pub ticket: String,
}

/// What `com.superlogical.terminal.format` returns: a terminal's screen and scrollback
/// in the form asked for, or the first part of it.
#[derive(Deserialize)]
pub(crate) struct Formatted {
    pub content: String,
    /// Names the rest, when there is more than `content` holds.
    #[serde(default)]
    pub id: Option<u64>,
}

/// What `com.superlogical.terminal.blob_read` returns: a further part of a result.
#[derive(Deserialize)]
pub(crate) struct BlobPart {
    pub data: String,
    #[serde(default)]
    pub eof: bool,
}

/// An event about the sessions of a server, as far as this client acts on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ServerEvent {
    SessionCreated(SessionSummary),
    SessionDestroyed {
        session_id: String,
    },
    SessionMoved {
        session_id: String,
        /// The session that now comes directly after it; none when it is last.
        before: Option<String>,
    },
    SessionLabel {
        session_id: String,
        label: String,
    },
    ViewChanged {
        session_id: String,
        revision: u64,
    },
    /// An event was too large to send, and this stands in its place. It may have been
    /// about one session.
    Dropped {
        session_id: Option<String>,
    },
    /// A terminal block reported something of itself.
    Block {
        block_id: String,
        /// What it reported, as reports of a session.
        events: Vec<SessionEvent>,
    },
    /// An event this client does nothing with.
    Other,
}

#[derive(Deserialize)]
struct Named {
    #[serde(rename = "type")]
    name: String,
    #[serde(default)]
    payload: serde_json::Value,
}

impl ServerEvent {
    /// Reads an event by its name and fields. One this client does not know, or whose
    /// fields are not as its name says, is one it does nothing with: newer servers add
    /// events.
    pub(crate) fn read(name: &str, payload: &serde_json::Value) -> Self {
        Self::try_read(name, payload).unwrap_or(ServerEvent::Other)
    }

    fn try_read(name: &str, payload: &serde_json::Value) -> Option<Self> {
        let text = |value: &serde_json::Value, field: &str| {
            let mut text = value.get(field)?.as_str()?.to_owned();
            cut(&mut text, TEXT_MAX_BYTES);
            Some(text)
        };
        Some(match name {
            "session_created" => {
                let session = payload.get("session")?;
                ServerEvent::SessionCreated(SessionSummary {
                    session_id: text(session, "session_id")?,
                    label: text(session, "label")?,
                })
            }
            "session_destroyed" => ServerEvent::SessionDestroyed {
                session_id: text(payload, "session_id")?,
            },
            "session_moved" => ServerEvent::SessionMoved {
                session_id: text(payload, "session_id")?,
                before: text(payload, "before_session_id"),
            },
            "event_dropped" => ServerEvent::Dropped {
                session_id: text(payload, "session_id"),
            },
            "session_event" => {
                let session_id = text(payload, "session_id")?;
                let inner = Named::deserialize(payload.get("event")?).ok()?;
                match inner.name.as_str() {
                    "session_label_changed" => ServerEvent::SessionLabel {
                        session_id,
                        label: text(&inner.payload, "label")?,
                    },
                    "session_view_changed" => ServerEvent::ViewChanged {
                        session_id,
                        revision: inner.payload.get("revision")?.as_u64()?,
                    },
                    "block_event" => {
                        let block = &inner.payload;
                        if text(block, "creator_name")? != TERMINAL_CREATOR {
                            return None;
                        }
                        let events = events::read(
                            &text(block, "name")?,
                            block.get("payload").unwrap_or(&serde_json::Value::Null),
                        );
                        if events.is_empty() {
                            return None;
                        }
                        ServerEvent::Block {
                            block_id: text(block, "block_id")?,
                            events,
                        }
                    }
                    _ => ServerEvent::Other,
                }
            }
            _ => ServerEvent::Other,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_view_reads_as_a_server_writes_one() {
        let view: View = serde_json::from_value(json!({
            "session_id": "session:1", "label": "work", "revision": 3,
            "active_window_id": "window:1",
            "windows": [{
                "window_id": "window:1", "label": "Window", "focused_block_id": "block:1",
                "layers": [{
                    "layer_id": "layer:1", "kind": "tiled", "bounds": [0, 0, 1, 1],
                    "layout": {"rect": [0, 0, 1, 1], "split": {
                        "split_id": "split:1", "direction": "horizontal", "ratio": 0.5,
                        "before": {"rect": [0, 0, 0.5, 1], "block_id": "block:1"},
                        "after": {"rect": [0.5, 0, 0.5, 1], "block_id": "block:2"}}},
                    "blocks": [
                        {"block_id": "block:1", "creator_name": "com.superlogical.terminal",
                         "flavor": "com.superlogical.terminal.shell", "label": "shell",
                         "host_id": "h", "owner": null},
                        {"block_id": "block:2", "creator_name": "com.superlogical.tunnel"}],
                    "a_field_from_a_newer_server": true
                }]
            }]
        }))
        .unwrap();
        let view = view.check().unwrap();
        let blocks: Vec<_> = view.blocks().collect();
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].is_terminal());
        assert!(!blocks[1].is_terminal());
        let split = view.windows[0].layers[0]
            .layout
            .as_ref()
            .unwrap()
            .split
            .as_ref()
            .unwrap();
        assert_eq!(split.before.block_id.as_deref(), Some("block:1"));
    }

    #[test]
    fn empty_lists_may_be_null_or_left_out() {
        let view: View = serde_json::from_value(json!({
            "revision": 1, "session_id": null, "windows": null
        }))
        .unwrap();
        assert!(view.windows.is_empty());
        let list: SessionList = serde_json::from_value(json!({ "sessions": null })).unwrap();
        assert!(list.sessions.is_empty());
    }

    fn nested(depth: usize) -> serde_json::Value {
        let mut node = json!({"rect": [0, 0, 1, 1], "block_id": "block:1"});
        for _ in 0..depth {
            node = json!({"rect": [0, 0, 1, 1], "split": {
                "split_id": null, "direction": "vertical", "ratio": 0.5,
                "before": node, "after": {"rect": [0, 0, 1, 1], "block_id": "block:2"}}});
        }
        json!({"revision": 1, "session_id": "s", "windows": [{
            "window_id": "w", "layers": [{"layer_id": "l", "kind": "tiled",
                "bounds": [0, 0, 1, 1], "layout": node, "blocks": []}]}]})
    }

    #[test]
    fn a_view_with_more_in_it_than_a_server_holds_is_refused() {
        let ok: View = serde_json::from_value(nested(SPLIT_DEPTH_MAX)).unwrap();
        assert!(ok.check().is_ok());
        // Deeper than this client follows, though not deeper than can be read.
        let deep: View = serde_json::from_value(nested(SPLIT_DEPTH_MAX + 1)).unwrap();
        assert_eq!(deep.check().unwrap_err().code(), "protocol_error");

        let windows: Vec<_> = (0..=WINDOWS_MAX)
            .map(|_| json!({"window_id": "w", "layers": []}))
            .collect();
        let many: View =
            serde_json::from_value(json!({"revision": 1, "session_id": "s", "windows": windows}))
                .unwrap();
        assert_eq!(many.check().unwrap_err().code(), "protocol_error");
    }

    #[test]
    fn what_is_too_long_is_cut_at_a_characters_edge() {
        let mut text = "é".repeat(TEXT_MAX_BYTES);
        cut(&mut text, TEXT_MAX_BYTES);
        assert_eq!(text.len(), TEXT_MAX_BYTES);
        let mut odd = format!("a{}", "é".repeat(TEXT_MAX_BYTES));
        cut(&mut odd, TEXT_MAX_BYTES);
        assert_eq!(odd.len(), TEXT_MAX_BYTES - 1);

        let summary = SessionSummary {
            session_id: "s".repeat(TEXT_MAX_BYTES * 2),
            label: "work".to_owned(),
        }
        .cut();
        assert_eq!(summary.session_id.len(), TEXT_MAX_BYTES);
    }

    #[test]
    fn events_are_read_by_name_and_the_unknown_are_passed_over() {
        assert_eq!(
            ServerEvent::read(
                "session_created",
                &json!({"session": {"session_id": "session:1", "label": "work"}})
            ),
            ServerEvent::SessionCreated(SessionSummary {
                session_id: "session:1".to_owned(),
                label: "work".to_owned()
            })
        );
        assert_eq!(
            ServerEvent::read(
                "session_event",
                &json!({"session_id": "session:1",
                        "event": {"type": "session_view_changed", "payload": {"revision": 9}}})
            ),
            ServerEvent::ViewChanged {
                session_id: "session:1".to_owned(),
                revision: 9
            }
        );
        assert_eq!(
            ServerEvent::read("session_moved", &json!({"session_id": "session:1"})),
            ServerEvent::SessionMoved {
                session_id: "session:1".to_owned(),
                before: None
            }
        );
        assert_eq!(
            ServerEvent::read(
                "event_dropped",
                &json!({"event_type": "block_event", "session_id": "session:1", "size": 9})
            ),
            ServerEvent::Dropped {
                session_id: Some("session:1".to_owned())
            }
        );
        assert_eq!(
            ServerEvent::read(
                "event_dropped",
                &json!({"event_type": "hosts_changed", "size": 9})
            ),
            ServerEvent::Dropped { session_id: None }
        );
        for (name, payload) in [
            ("something_new", json!({})),
            ("session_created", json!({"session": 3})),
            ("session_destroyed", json!({"session_id": 7})),
            ("session_event", json!({"session_id": "s", "event": "no"})),
            (
                "session_event",
                json!({"session_id": "s", "event": {"type": "session_view_changed", "payload": {"revision": -1}}}),
            ),
        ] {
            assert_eq!(
                ServerEvent::read(name, &payload),
                ServerEvent::Other,
                "{name}"
            );
        }
    }

    #[test]
    fn a_secret_is_not_written_out() {
        assert_eq!(format!("{:?}", Secret("token".to_owned())), "Secret(…)");
    }
}
