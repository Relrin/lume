//! The WebSocket every connection to a server runs over.
//!
//! A server serves HTTP on its endpoint, and each connection is a WebSocket at a path
//! of its own, speaking one subprotocol that the server must agree to.

use serde_json::Value;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{WebSocketStream, client_async_with_config};

use crate::error::RexError;
use crate::text::plain;
use crate::transport::Io;
use crate::tuning::CONNECT_TIMEOUT;

/// Shakes hands for a WebSocket at `path` over a stream to a server. `endpoint` names
/// the server in what is said of failures, and `authority` is what the server is asked
/// for by: its host, and its port where that is not the usual one. No message larger
/// than `max_message_bytes` is taken from the server afterwards.
pub(crate) async fn open<S: Io>(
    io: S,
    endpoint: &str,
    authority: &str,
    path: &str,
    subprotocol: &str,
    max_message_bytes: usize,
) -> Result<WebSocketStream<S>, RexError> {
    let protocol = |what: String| RexError::Protocol(what);
    // The stream is open already, and encrypted where it is to be: the address only
    // says which server is asked for.
    let mut request = format!("ws://{authority}{path}")
        .into_client_request()
        .map_err(|error| protocol(error.to_string()))?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        subprotocol
            .parse()
            .map_err(|_| protocol("the subprotocol is not a header value".to_owned()))?,
    );
    let config = WebSocketConfig::default()
        .max_message_size(Some(max_message_bytes))
        .max_frame_size(Some(max_message_bytes));
    let shaken = tokio::time::timeout(
        CONNECT_TIMEOUT,
        client_async_with_config(request, io, Some(config)),
    )
    .await
    .map_err(|_| RexError::Timeout("the connection".to_owned()))?;
    let (socket, response) = shaken.map_err(|error| handshake_error(endpoint, error))?;
    let spoken = response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|value| value.to_str().ok());
    if spoken != Some(subprotocol) {
        return Err(protocol(format!(
            "the server did not agree to speak {subprotocol}"
        )));
    }
    Ok(socket)
}

/// What a failed handshake says: a server that does not let a caller in answers every
/// request with status 403 and a JSON body that says why.
fn handshake_error(endpoint: &str, error: WsError) -> RexError {
    match error {
        WsError::Http(response) => {
            let status = response.status().as_u16();
            let said = response
                .body()
                .as_deref()
                .and_then(|body| serde_json::from_slice::<Value>(body).ok())
                .and_then(|body| body.get("error")?.as_str().map(str::to_owned));
            let reason = plain(&said.unwrap_or_else(|| format!("status {status}")));
            if status == 403 {
                RexError::Refused {
                    endpoint: endpoint.to_owned(),
                    reason,
                }
            } else {
                RexError::Protocol(format!("the server answered the handshake with {reason}"))
            }
        }
        WsError::Io(source) => RexError::Unreachable {
            endpoint: endpoint.to_owned(),
            source,
        },
        other => RexError::Protocol(plain(&format!("the handshake failed: {other}"))),
    }
}
