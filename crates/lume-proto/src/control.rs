//! Control-plane messages: JSON objects carried in `Control` frames on stream 0.
//!
//! A message is exactly one of three things:
//!
//! ```json
//! {"request":  {"id": 7, "body": {"type": "kill", "session": "00ab…"}}}
//! {"response": {"id": 7, "ok": {"type": "killed"}}}
//! {"response": {"id": 7, "err": {"code": "not_found", "message": "…"}}}
//! {"notify":   {"type": "session_exited", "session": "00ab…", "status": {"code": 0}}}
//! ```
//!
//! Unknown fields are refused everywhere. The two sides agree on one protocol version
//! before anything else is said, so a field one side does not know is an error to report,
//! not something to skip.

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, ProtoError};
use crate::types::{
    BackendCaps, ClientFacts, ExitStatus, SessionId, SessionInfo, SpawnSpec, StreamId, TermSize,
};

/// One control-plane message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    Request(Request),
    Response(Response),
    Notify(Notification),
}

/// A request. The response carries the same `id`; the sender chooses ids and does not
/// reuse one while its request is unanswered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: u64,
    pub body: RequestBody,
}

/// What kind of program the client is. Informational: a host may log it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    /// The application, on behalf of its windows.
    App,
    /// The command-line control tool.
    Ctl,
    /// A test client.
    Test,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequestBody {
    /// Must be the first request on a connection.
    Hello {
        /// The newest protocol version the client speaks.
        proto: u32,
        /// The oldest protocol version the client speaks.
        proto_min: u32,
        /// The terminal engine version the client runs, if it runs one.
        vt_version: Option<String>,
        client_kind: ClientKind,
    },
    Spawn {
        spec: SpawnSpec,
    },
    List {},
    /// Opens `stream` as an attachment to `session`. The client chooses the stream id: at
    /// least 1 and never used before on this connection. The stream starts with no credit.
    Attach {
        session: SessionId,
        stream: StreamId,
        facts: ClientFacts,
    },
    Kill {
        session: SessionId,
    },
}

/// The answer to a request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawResponse", into = "RawResponse")]
pub struct Response {
    pub id: u64,
    pub result: Result<ResponseBody, ProtoError>,
}

/// A response as it is spelt in JSON: `ok` or `err`, never both and never neither.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawResponse {
    id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ok: Option<ResponseBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    err: Option<ProtoError>,
}

impl TryFrom<RawResponse> for Response {
    type Error = &'static str;

    fn try_from(raw: RawResponse) -> Result<Self, Self::Error> {
        let result = match (raw.ok, raw.err) {
            (Some(body), None) => Ok(body),
            (None, Some(error)) => Err(error),
            _ => return Err("a response carries exactly one of `ok` and `err`"),
        };
        Ok(Self { id: raw.id, result })
    }
}

impl From<Response> for RawResponse {
    fn from(response: Response) -> Self {
        let (ok, err) = match response.result {
            Ok(body) => (Some(body), None),
            Err(error) => (None, Some(error)),
        };
        Self {
            id: response.id,
            ok,
            err,
        }
    }
}

/// How the host brings a newly attached client up to date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HydrateMode {
    /// Nothing is sent: the client starts blank and follows the output from here.
    None,
    /// A `Hydrate` frame with a serialised terminal precedes the output.
    Snapshot,
    /// A `Hydrate` frame with output to replay precedes the output.
    VtReplay,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResponseBody {
    HelloAck {
        /// The protocol version both sides now speak.
        proto: u32,
        host_version: String,
        /// The terminal engine version the host runs, if it runs one.
        vt_version: Option<String>,
        caps: BackendCaps,
        /// The largest frame payload the host sends and accepts.
        max_payload: u32,
        pid: u32,
    },
    Spawned {
        session: SessionId,
    },
    Sessions {
        sessions: Vec<SessionInfo>,
    },
    /// The attachment is open. This response is written before any frame of its stream.
    Attached {
        /// The terminal's size now.
        size: TermSize,
        hydrate: HydrateMode,
        /// Whether growing this terminal pulls lines back out of scrollback. A terminal
        /// that repaints its own screen after a resize does not, and a client that pulled
        /// them anyway would show those lines twice.
        resize_pulls_scrollback: bool,
    },
    Killed {},
}

/// Something the host says without being asked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Notification {
    SessionExited {
        session: SessionId,
        status: ExitStatus,
    },
    /// The host is about to close the connection because of this.
    Fatal { code: ErrorCode, message: String },
}

/// The version a host speaking only `host` uses with a client speaking `proto_min..=proto`,
/// or `None` when they share none.
pub fn negotiate(host: u32, proto: u32, proto_min: u32) -> Option<u32> {
    (proto_min..=proto).contains(&host).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<Control, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn requests_have_the_documented_shape() {
        let kill = Control::Request(Request {
            id: 7,
            body: RequestBody::Kill {
                session: SessionId::new(0xab).unwrap(),
            },
        });
        let json = r#"{"request":{"id":7,"body":{"type":"kill","session":"00000000000000ab"}}}"#;
        assert_eq!(serde_json::to_string(&kill).unwrap(), json);
        assert_eq!(parse(json).unwrap(), kill);

        let list = Control::Request(Request {
            id: 1,
            body: RequestBody::List {},
        });
        let json = r#"{"request":{"id":1,"body":{"type":"list"}}}"#;
        assert_eq!(serde_json::to_string(&list).unwrap(), json);
        assert_eq!(parse(json).unwrap(), list);
    }

    #[test]
    fn responses_carry_exactly_one_of_ok_and_err() {
        let ok = Control::Response(Response {
            id: 7,
            result: Ok(ResponseBody::Killed {}),
        });
        let json = r#"{"response":{"id":7,"ok":{"type":"killed"}}}"#;
        assert_eq!(serde_json::to_string(&ok).unwrap(), json);
        assert_eq!(parse(json).unwrap(), ok);

        let err = Control::Response(Response {
            id: 7,
            result: Err(ProtoError::new(ErrorCode::NotFound, "no such session")),
        });
        let json =
            r#"{"response":{"id":7,"err":{"code":"not_found","message":"no such session"}}}"#;
        assert_eq!(serde_json::to_string(&err).unwrap(), json);
        assert_eq!(parse(json).unwrap(), err);

        assert!(parse(r#"{"response":{"id":7}}"#).is_err());
        assert!(
            parse(
                r#"{"response":{"id":7,"ok":{"type":"killed"},"err":{"code":"internal","message":""}}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn unknown_fields_are_refused_at_every_level() {
        for json in [
            r#"{"request":{"id":1,"body":{"type":"list"},"extra":1}}"#,
            r#"{"request":{"id":1,"body":{"type":"list","extra":1}}}"#,
            r#"{"request":{"id":1,"body":{"type":"kill","session":"00000000000000ab","extra":1}}}"#,
            r#"{"response":{"id":1,"ok":{"type":"killed","extra":1}}}"#,
            r#"{"response":{"id":1,"ok":{"type":"killed"},"extra":1}}"#,
            r#"{"notify":{"type":"fatal","code":"internal","message":"","extra":1}}"#,
            r#"{"request":{"id":1,"body":{"type":"no_such_request"}}}"#,
        ] {
            assert!(parse(json).is_err(), "accepted: {json}");
        }
    }

    #[test]
    fn a_host_answers_with_its_own_version_only_inside_the_clients_range() {
        assert_eq!(negotiate(1, 1, 1), Some(1));
        assert_eq!(negotiate(2, 3, 2), Some(2));
        assert_eq!(negotiate(1, 3, 2), None);
        assert_eq!(negotiate(4, 3, 2), None);
    }
}
