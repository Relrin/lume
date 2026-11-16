//! `lume ctl`: one request to the running application, and its answer on standard output.
//!
//! The answer is one JSON object on one line. A request that was carried out prints
//! `{"ok":true,"effect":…}` with the fields of its answer beside it; one that was not
//! prints `{"ok":false,"code":…,"message":…}`. The exit status says the same in a form a
//! shell can branch on.
//!
//! One request has more than an answer: `events` follows what the sessions report, and
//! prints each report as a line of its own after the answer, for as long as it is left
//! to run.

use std::time::Duration;

use lume_proto::app::{AppError, AppReply, AppReplyBody, AppRequestBody, codes};
use serde_json::{Value, json};

use crate::args::Ctl;
use crate::client::{ANSWER_TIMEOUT, Client, Failure, endpoint};

/// The request was carried out.
pub const EXIT_OK: u8 = 0;
/// The request was refused or failed.
pub const EXIT_FAILED: u8 = 1;
/// A wait ran out of time.
pub const EXIT_TIMEOUT: u8 = 3;
/// Nothing serves the endpoint.
pub const EXIT_NOT_RUNNING: u8 = 4;

/// How much longer than a wait's own limit the connection is given: the application
/// keeps the time, and its answer that the time ran out must be able to arrive.
const WAIT_SLACK: Duration = Duration::from_secs(10);

fn failure(code: &str, message: impl std::fmt::Display) -> Value {
    json!({ "ok": false, "code": code, "message": message.to_string() })
}

/// The object printed for a request that was carried out.
fn success(reply: &AppReply) -> Value {
    let mut object = match serde_json::to_value(&reply.body) {
        Ok(Value::Object(object)) => object,
        _ => serde_json::Map::new(),
    };
    object.insert("ok".to_owned(), Value::Bool(true));
    object.insert("effect".to_owned(), Value::String(reply.effect.clone()));
    Value::Object(object)
}

/// How long the application may take to answer `request`.
fn answer_limit(request: &AppRequestBody, timeout: Duration) -> Duration {
    match request {
        AppRequestBody::WaitExit { .. } | AppRequestBody::WaitText { .. } => timeout + WAIT_SLACK,
        _ => ANSWER_TIMEOUT,
    }
}

/// The request to send: the one the arguments gave, with the intent read from
/// standard input put in if that is where it was said to be.
fn request(ctl: &Ctl) -> Result<AppRequestBody, Failure> {
    if !ctl.intent_from_input {
        return Ok(ctl.request.clone());
    }
    let mut written = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut written).map_err(Failure::Broken)?;
    let intent = serde_json::from_str(&written).map_err(|error| {
        Failure::Refused(AppError::new(
            codes::BAD_REQUEST,
            format!("the intent on standard input is not JSON: {error}"),
        ))
    })?;
    Ok(AppRequestBody::LayoutApply { intent })
}

async fn ask(ctl: &Ctl) -> Result<(AppReply, Client), Failure> {
    let request = request(ctl)?;
    let endpoint = endpoint(ctl.endpoint.as_deref()).map_err(Failure::Broken)?;
    let mut client = Client::connect(&endpoint).await?;
    let limit = answer_limit(&request, ctl.timeout);
    let reply = client.ask(request, limit).await?;
    Ok((reply, client))
}

/// Prints what the sessions report, a line each, until the application closes the
/// connection or `limit` has passed.
async fn follow(client: &mut Client, limit: Option<Duration>) -> Result<(), Failure> {
    let reports = async {
        while let Some(event) = client.event().await.map_err(Failure::Broken)? {
            match serde_json::to_string(&event) {
                Ok(line) => println!("{line}"),
                Err(error) => return Err(Failure::Broken(std::io::Error::other(error))),
            }
        }
        Ok(())
    };
    match limit {
        // Running out of the time that was asked for is how this ends well.
        Some(limit) => tokio::time::timeout(limit, reports).await.unwrap_or(Ok(())),
        None => reports.await,
    }
}

/// Sends the request, prints the answer and returns the exit status.
pub async fn run(ctl: Ctl) -> u8 {
    let asked = match ask(&ctl).await {
        Ok((reply, mut client)) => {
            match (&reply.body, ctl.plain) {
                (AppReplyBody::Screen { lines, .. }, true) => {
                    for line in lines {
                        println!("{line}");
                    }
                }
                _ => println!("{}", success(&reply)),
            }
            match reply.body {
                AppReplyBody::Following {} => follow(&mut client, ctl.follow_for).await,
                _ => Ok(()),
            }
        }
        Err(failure) => Err(failure),
    };
    match asked {
        Ok(()) => EXIT_OK,
        Err(Failure::Refused(error)) => {
            println!("{}", failure(&error.code, &error.message));
            if error.code == codes::TIMEOUT {
                EXIT_TIMEOUT
            } else {
                EXIT_FAILED
            }
        }
        Err(Failure::NotRunning(error)) => {
            println!(
                "{}",
                failure(
                    "not_running",
                    format!("the application is not running ({error})")
                )
            );
            EXIT_NOT_RUNNING
        }
        Err(Failure::Broken(error)) => {
            println!("{}", failure("disconnected", error));
            EXIT_FAILED
        }
    }
}

#[cfg(test)]
mod tests {
    use lume_proto::TermSize;

    use super::*;

    #[test]
    fn an_answer_is_printed_flat_with_its_effect() {
        let reply = AppReply {
            effect: "the terminal is now 90 by 20".to_owned(),
            body: AppReplyBody::Resized {
                size: TermSize::new(90, 20).unwrap(),
            },
        };
        assert_eq!(
            success(&reply),
            json!({
                "ok": true,
                "effect": "the terminal is now 90 by 20",
                "type": "resized",
                "size": { "cols": 90, "rows": 20 },
            })
        );
    }

    #[test]
    fn a_failure_is_printed_with_its_code() {
        assert_eq!(
            failure("timeout", "the text never appeared"),
            json!({ "ok": false, "code": "timeout", "message": "the text never appeared" })
        );
    }

    #[test]
    fn a_wait_is_given_its_own_time_and_a_little_more() {
        let wait = AppRequestBody::WaitExit {
            pane: None,
            session: None,
            timeout_ms: 0,
        };
        assert_eq!(
            answer_limit(&wait, Duration::from_secs(300)),
            Duration::from_secs(310)
        );
        assert_eq!(
            answer_limit(&AppRequestBody::List {}, Duration::from_secs(300)),
            ANSWER_TIMEOUT
        );
    }
}
