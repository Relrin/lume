//! The values both planes carry: identifiers, sizes, colours, client facts and session
//! descriptions.

use std::fmt;
use std::num::NonZeroU64;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// Identifies one stream on a connection. Stream 0 is the control plane.
pub type StreamId = u32;

/// Identifies a session within one host.
///
/// Ids are random rather than counted, so an id kept by a client cannot name a different
/// session after the host restarts. In JSON an id is sixteen lowercase hexadecimal digits:
/// a JavaScript number cannot hold every 64-bit value.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(NonZeroU64);

impl SessionId {
    /// Makes an id from its numeric value. Zero is not an id.
    pub fn new(raw: u64) -> Option<Self> {
        NonZeroU64::new(raw).map(Self)
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionId({self})")
    }
}

impl FromStr for SessionId {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        const WHAT: &str = "a session id is sixteen lowercase hexadecimal digits and not zero";
        let well_formed =
            s.len() == 16 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !well_formed {
            return Err(WHAT);
        }
        u64::from_str_radix(s, 16)
            .ok()
            .and_then(Self::new)
            .ok_or(WHAT)
    }
}

impl Serialize for SessionId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SessionId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(de::Error::custom)
    }
}

/// Names a session backend. A pane refers to its session by backend id and session id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BackendId(pub String);

impl fmt::Display for BackendId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A grid size outside `1..=TermSize::MAX` in either direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a terminal size of {cols}x{rows} is outside 1..={max}", max = TermSize::MAX)]
pub struct InvalidSize {
    pub cols: u16,
    pub rows: u16,
}

/// A terminal grid size in cells. Both dimensions are at least 1 and at most
/// [`TermSize::MAX`], so code that receives one never has to handle an empty or absurd grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawTermSize", into = "RawTermSize")]
pub struct TermSize {
    cols: u16,
    rows: u16,
}

impl TermSize {
    /// The largest grid dimension accepted on the wire.
    pub const MAX: u16 = 4096;

    pub fn new(cols: u16, rows: u16) -> Result<Self, InvalidSize> {
        let valid = |n: u16| (1..=Self::MAX).contains(&n);
        if valid(cols) && valid(rows) {
            Ok(Self { cols, rows })
        } else {
            Err(InvalidSize { cols, rows })
        }
    }

    pub fn cols(self) -> u16 {
        self.cols
    }

    pub fn rows(self) -> u16 {
        self.rows
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTermSize {
    cols: u16,
    rows: u16,
}

impl TryFrom<RawTermSize> for TermSize {
    type Error = InvalidSize;

    fn try_from(raw: RawTermSize) -> Result<Self, Self::Error> {
        Self::new(raw.cols, raw.rows)
    }
}

impl From<TermSize> for RawTermSize {
    fn from(size: TermSize) -> Self {
        Self {
            cols: size.cols,
            rows: size.rows,
        }
    }
}

/// The size of one cell in device pixels. Zero means the client has not measured it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellPx {
    pub w: u16,
    pub h: u16,
}

/// An opaque colour: red, green, blue. In JSON it is `"#rrggbb"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb(pub [u8; 3]);

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b] = self.0;
        write!(f, "#{r:02x}{g:02x}{b:02x}")
    }
}

impl FromStr for Rgb {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        const WHAT: &str = "a colour is `#` followed by six lowercase hexadecimal digits";
        let digits = s.strip_prefix('#').ok_or(WHAT)?;
        let well_formed = digits.len() == 6
            && digits
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !well_formed {
            return Err(WHAT);
        }
        let value = u32::from_str_radix(digits, 16).map_err(|_| WHAT)?;
        let [_, r, g, b] = value.to_be_bytes();
        Ok(Self([r, g, b]))
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(de::Error::custom)
    }
}

/// The colours a client draws with: what a program learns when it asks the terminal for
/// its foreground, background, cursor or one of the sixteen ANSI colours.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Palette {
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub ansi: [Rgb; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorScheme {
    Light,
    Dark,
}

/// What a client tells the host about the view it shows a session in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientFacts {
    pub size: TermSize,
    pub cell_px: CellPx,
    pub palette: Palette,
    pub scheme: ColorScheme,
    pub focused: bool,
    /// Whether the view is on screen, as opposed to in a hidden tab or window.
    pub visible: bool,
}

/// How a session's program ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitStatus {
    /// The program returned this exit code.
    Code(u32),
    /// The program was ended by this signal.
    Signal(u32),
    /// The program ended but the platform did not say how.
    Unknown,
}

/// What a session runs. More ways to start a session are added as variants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Launcher {
    /// A program on this machine, started directly in a terminal.
    Shell {
        program: String,
        args: Vec<String>,
        /// Start the program as a login shell. Only Unix shells have the notion.
        login: bool,
    },
    /// The default shell of a Linux distribution on this Windows machine.
    Wsl {
        distro: String,
        /// The distribution's user to start as. `None` is the distribution's default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user: Option<String>,
    },
    /// A shell on another machine, reached with this machine's OpenSSH client. The
    /// client reads the user's own configuration, so `host` may be a name from it.
    Ssh {
        host: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        /// Arguments given to the client before the host.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        extra_args: Vec<String>,
        /// The client program to run instead of the one the host finds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client: Option<String>,
    },
}

impl Launcher {
    /// What kind of place the session is in, as one lowercase word.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Shell { .. } => "shell",
            Self::Wsl { .. } => "wsl",
            Self::Ssh { .. } => "ssh",
        }
    }
}

/// Everything needed to start a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnSpec {
    pub launcher: Launcher,
    /// The working directory. `None` leaves the choice to the host.
    pub cwd: Option<String>,
    /// Variables set on top of the host's own environment.
    pub env: Vec<(String, String)>,
    pub size: TermSize,
}

/// Counters a host keeps per session. They describe the flow of output, never its content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionStats {
    /// Bytes read from the terminal so far.
    pub read_bytes: u64,
    /// Bytes sent to a client so far.
    pub sent_bytes: u64,
    /// Bytes read but not yet sent.
    pub pending_bytes: u32,
    /// Whether the host has stopped reading the terminal because nothing can take more.
    pub paused: bool,
}

/// One entry of a host's session list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionInfo {
    pub id: SessionId,
    pub program: String,
    pub pid: Option<u32>,
    pub size: TermSize,
    /// When the session started, in milliseconds since the Unix epoch.
    pub created_unix_ms: u64,
    pub attached: bool,
    /// Set once the program has ended and until the session is removed.
    pub exit: Option<ExitStatus>,
    pub stats: SessionStats,
}

/// Which side answers a program's questions to the terminal (device attributes, cursor
/// position, colours). Exactly one side does, or a program would get two answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Responder {
    Client,
    Backend,
}

/// The ways a backend can bring a newly attached client up to date.
/// Neither set means it cannot: the client starts blank and follows the output from there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HydrateKinds {
    pub snapshot: bool,
    pub vt_replay: bool,
}

/// What differs between backends. Clients branch on these, never on which backend it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendCaps {
    /// Sessions outlive the client connection.
    pub persistent: bool,
    /// A session accepts several attachments at once.
    pub multi_client: bool,
    /// The backend, not the client, owns the arrangement of panes.
    pub server_layout: bool,
    pub responder: Responder,
    pub hydrate: HydrateKinds,
}

/// Something a session reports besides its output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum SessionEvent {
    Title {
        title: String,
    },
    /// The working directory the program reported, as a URL.
    Cwd {
        url: String,
    },
    Bell,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_sixteen_hex_digits_in_json() {
        let id = SessionId::new(0x00ab_cdef_0123_4567).unwrap();
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"00abcdef01234567\"");
        assert_eq!(
            serde_json::from_str::<SessionId>("\"00abcdef01234567\"").unwrap(),
            id
        );
    }

    #[test]
    fn session_id_rejects_zero_and_other_spellings() {
        for text in [
            "0000000000000000",
            "abc",
            "00ABCDEF01234567",
            "+0abcdef01234567",
            "",
        ] {
            assert!(text.parse::<SessionId>().is_err(), "{text:?} was accepted");
        }
    }

    #[test]
    fn term_size_rejects_empty_and_oversized_grids() {
        assert!(TermSize::new(80, 24).is_ok());
        assert!(TermSize::new(TermSize::MAX, TermSize::MAX).is_ok());
        assert!(TermSize::new(0, 24).is_err());
        assert!(TermSize::new(80, 0).is_err());
        assert!(TermSize::new(TermSize::MAX + 1, 24).is_err());
        assert!(serde_json::from_str::<TermSize>(r#"{"cols":0,"rows":24}"#).is_err());
        assert!(serde_json::from_str::<TermSize>(r#"{"cols":80,"rows":24,"x":1}"#).is_err());
    }

    #[test]
    fn colour_is_a_hash_and_six_digits_in_json() {
        let colour = Rgb([0x12, 0xab, 0xff]);
        assert_eq!(serde_json::to_string(&colour).unwrap(), "\"#12abff\"");
        assert_eq!(serde_json::from_str::<Rgb>("\"#12abff\"").unwrap(), colour);
        for text in ["12abff", "#12ABFF", "#12abf", "#12abff0", "#+2abff"] {
            assert!(text.parse::<Rgb>().is_err(), "{text:?} was accepted");
        }
    }

    #[test]
    fn exit_status_has_three_json_shapes() {
        assert_eq!(
            serde_json::to_string(&ExitStatus::Code(3)).unwrap(),
            r#"{"code":3}"#
        );
        assert_eq!(
            serde_json::to_string(&ExitStatus::Signal(9)).unwrap(),
            r#"{"signal":9}"#
        );
        assert_eq!(
            serde_json::to_string(&ExitStatus::Unknown).unwrap(),
            r#""unknown""#
        );
    }
}
