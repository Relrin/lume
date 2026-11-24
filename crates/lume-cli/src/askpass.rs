//! Asking the user for the SSH client.
//!
//! The client has no terminal to ask in when the application runs it. It runs this
//! program instead, with its words as the one argument, and takes what is printed for
//! the answer. The application it belongs to puts the question to the user, and is found
//! through the environment it gave the client. Without an answer nothing is printed.

use std::io::Write as _;
use std::process::ExitCode;
use std::time::Duration;

use lume_proto::Secret;
use lume_proto::app::{ASKPASS_ENDPOINT_VAR, ASKPASS_TOKEN_VAR, AppReplyBody, AppRequestBody};
use zeroize::Zeroizing;

use crate::client::{Client, endpoint};

/// How long an answer is waited for. The application gives up on a user who says
/// nothing long before.
const ANSWER_LIMIT: Duration = Duration::from_secs(600);
/// How the client says what kind of question it asks.
const HINT_VAR: &str = "SSH_ASKPASS_PROMPT";

/// What the SSH client wants to know.
pub struct Asked {
    endpoint: String,
    token: Secret,
    prompt: String,
    hint: Option<String>,
}

/// What is asked, if it is the SSH client that runs this program.
pub fn asked() -> Option<Asked> {
    let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    Some(Asked {
        endpoint: var(ASKPASS_ENDPOINT_VAR)?,
        token: Secret::new(var(ASKPASS_TOKEN_VAR)?),
        prompt: std::env::args_os()
            .nth(1)
            .map(|words| words.to_string_lossy().into_owned())
            .unwrap_or_default(),
        hint: var(HINT_VAR),
    })
}

/// The user's answer, if there is one.
async fn answer(asked: Asked) -> Option<Secret> {
    let endpoint = endpoint(Some(&asked.endpoint)).ok()?;
    let mut client = Client::connect(&endpoint).await.ok()?;
    let request = AppRequestBody::Askpass {
        token: asked.token,
        prompt: asked.prompt,
        hint: asked.hint,
    };
    match client.ask(request, ANSWER_LIMIT).await.ok()?.body {
        AppReplyBody::Secret { secret } => secret,
        _ => None,
    }
}

/// Asks, and prints the answer.
pub fn run(asked: Asked) -> ExitCode {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    // The client shows the user nothing of what is said here, so nothing is.
    let Ok(Some(secret)) = runtime.map(|runtime| runtime.block_on(answer(asked))) else {
        return ExitCode::FAILURE;
    };
    let mut line = Zeroizing::new(Vec::with_capacity(secret.expose().len() + 1));
    line.extend_from_slice(secret.expose().as_bytes());
    line.push(b'\n');
    let mut out = std::io::stdout().lock();
    match out.write_all(&line).and_then(|()| out.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
