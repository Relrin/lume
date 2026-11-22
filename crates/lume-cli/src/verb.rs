//! Verbs plugins add: `lume <verb> [arguments]`.
//!
//! The words are sent to the running application as they were written, with the
//! directory this was run in and whatever was piped in. The application finds the verb
//! at the front of the words and runs the plugin's command for it; what that prints
//! arrives a piece at a time and is printed at once, and the command's status is this
//! program's.
//!
//! Interrupting this program closes the connection, which is how the application
//! learns that the verb is to stop.

use std::io::{IsTerminal as _, Read as _, Write as _};

use lume_proto::app::{AppReplyBody, AppRequestBody, codes};

use crate::client::{ANSWER_TIMEOUT, Client, Failure, Sent, endpoint};
use crate::ctl::{EXIT_FAILED, EXIT_NOT_RUNNING};

/// The exit status when no plugin adds the verb: the arguments were not understood.
const EXIT_UNKNOWN: u8 = 2;

/// The most text that may be piped to a verb. The application refuses more.
const STDIN_MAX: usize = 512 * 1024;

/// What was piped in, if anything was: nothing when this is run at a terminal.
fn piped() -> Result<Option<String>, String> {
    let input = std::io::stdin();
    if input.is_terminal() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    input
        .lock()
        .take(STDIN_MAX as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("what was piped in could not be read: {error}"))?;
    if bytes.len() > STDIN_MAX {
        return Err(format!(
            "a verb is given {} KiB at most, and more was piped in",
            STDIN_MAX / 1024
        ));
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "what is piped to a verb must be text, as UTF-8".to_owned())
}

/// Says that no plugin adds the verb, and which verbs there are.
async fn unknown(client: &mut Client, words: &[String]) {
    let first = words.first().map_or("", String::as_str);
    let mut said = format!("lume: {first} is not something lume knows\n");
    let verbs = match client.ask(AppRequestBody::Verbs {}, ANSWER_TIMEOUT).await {
        Ok(reply) => match reply.body {
            AppReplyBody::Verbs { verbs } => verbs.as_array().cloned().unwrap_or_default(),
            _ => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    if verbs.is_empty() {
        said.push_str("No add-on adds a verb. lume --help says what lume itself knows.\n");
    } else {
        said.push_str("The verbs add-ons add:\n");
        let width = verbs
            .iter()
            .filter_map(|verb| verb["verb"].as_str())
            .map(|verb| verb.chars().count())
            .max()
            .unwrap_or(0);
        for verb in &verbs {
            let (name, summary, plugin) = (
                verb["verb"].as_str().unwrap_or_default(),
                verb["summary"].as_str().unwrap_or_default(),
                verb["plugin"].as_str().unwrap_or_default(),
            );
            said.push_str(&format!("  lume {name:width$}  {summary} ({plugin})\n"));
        }
        said.push_str("lume --help says what lume itself knows.\n");
    }
    eprint!("{said}");
}

/// Runs the verb the words begin with and returns the status to exit with.
pub async fn run(endpoint_name: Option<&str>, words: Vec<String>) -> u8 {
    let first = words.first().cloned().unwrap_or_default();
    let stdin = match piped() {
        Ok(stdin) => stdin,
        Err(problem) => {
            eprintln!("lume: {problem}");
            return EXIT_FAILED;
        }
    };
    let cwd = std::env::current_dir()
        .map(|dir| dir.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut client = match endpoint(endpoint_name).map_err(Failure::Broken) {
        Ok(endpoint) => Client::connect(&endpoint).await,
        Err(failure) => Err(failure),
    };
    let client = match &mut client {
        Ok(client) => client,
        Err(Failure::NotRunning(_)) => {
            eprintln!(
                "lume: {first} is not one of lume's own commands. An add-on may add it, but \
                 the application is not running, and it is the application that runs an \
                 add-on's verbs."
            );
            return EXIT_NOT_RUNNING;
        }
        Err(Failure::Broken(error)) => {
            eprintln!("lume: the application could not be reached: {error}");
            return EXIT_FAILED;
        }
        Err(Failure::Refused(error)) => {
            eprintln!("lume: {}", error.message);
            return EXIT_FAILED;
        }
    };

    let request = AppRequestBody::Verb {
        words: words.clone(),
        cwd,
        stdin,
    };
    match client.ask(request, ANSWER_TIMEOUT).await {
        Ok(reply) if matches!(reply.body, AppReplyBody::Running { .. }) => {}
        Ok(_) => {
            eprintln!("lume: the application answered something else than that it runs {first}");
            return EXIT_FAILED;
        }
        Err(Failure::Refused(error)) if error.code == codes::NOT_FOUND => {
            unknown(client, &words).await;
            return EXIT_UNKNOWN;
        }
        Err(Failure::Refused(error)) => {
            eprintln!("lume: {}", error.message);
            return EXIT_FAILED;
        }
        Err(Failure::NotRunning(error) | Failure::Broken(error)) => {
            eprintln!("lume: {first} could not be asked for: {error}");
            return EXIT_FAILED;
        }
    }

    loop {
        match client.next().await {
            Ok(Some(Sent::Output(output))) => {
                // Printed as it comes: a verb may run for a long time. Output that can
                // no longer be written ends this, and with it the verb.
                let written = if output.error {
                    let mut err = std::io::stderr().lock();
                    err.write_all(output.text.as_bytes())
                        .and_then(|()| err.flush())
                } else {
                    let mut out = std::io::stdout().lock();
                    out.write_all(output.text.as_bytes())
                        .and_then(|()| out.flush())
                };
                if written.is_err() {
                    return EXIT_FAILED;
                }
            }
            Ok(Some(Sent::Done(done))) => return done.code,
            Ok(Some(Sent::Event(_))) => {}
            Ok(None) => {
                eprintln!("lume: the application closed the connection before {first} ended");
                return EXIT_FAILED;
            }
            Err(error) => {
                eprintln!("lume: the connection to the application failed: {error}");
                return EXIT_FAILED;
            }
        }
    }
}
