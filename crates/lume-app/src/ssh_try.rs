//! Whether a machine can be signed in to over SSH, found out before a profile for it is
//! written.
//!
//! The client runs out of sight and asks nothing: an account that needs a password, or
//! a machine whose key is not known, is said to, and is settled in a terminal.

use std::process::Stdio;
use std::sync::{Mutex, PoisonError};

use lume_proto::{DisconnectReason, ExitStatus};
use lume_session::Launcher;
use lume_session::ssh::{Tail, check_word, classify, client};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::error::CommandError;
use crate::tuning::{SSH_TRY_CONNECT_SECONDS, SSH_TRY_WAIT};

/// What the other machine's shell is told to do: to end.
const ASK: &str = "exit";

/// The arguments the client is run with. The destination follows `--`, so nothing typed
/// is read as an option.
fn arguments(
    host: &str,
    user: Option<&str>,
    port: Option<u16>,
    extra_args: &[String],
) -> Result<Vec<String>, CommandError> {
    let word = |what: &str, word: &str| {
        check_word(what, word).map_err(|error| CommandError::bad_request(error.message))
    };
    word("host", host)?;
    if let Some(user) = user {
        word("user's name", user)?;
    }
    let mut args = Vec::new();
    for option in [
        "BatchMode=yes".to_owned(),
        format!("ConnectTimeout={SSH_TRY_CONNECT_SECONDS}"),
        // A connection that is there already says nothing of signing in.
        "ControlPath=none".to_owned(),
    ] {
        args.extend(["-o".to_owned(), option]);
    }
    args.extend(["-T".to_owned(), "-n".to_owned()]);
    args.extend(extra_args.iter().cloned());
    if let Some(port) = port {
        args.extend(["-p".to_owned(), port.to_string()]);
    }
    let destination = match user {
        Some(user) => format!("{user}@{host}"),
        None => host.to_owned(),
    };
    args.extend(["--".to_owned(), destination, ASK.to_owned()]);
    Ok(args)
}

fn failure(reason: DisconnectReason, detail: Option<String>) -> CommandError {
    let code = match reason {
        DisconnectReason::AuthFailed => "auth",
        DisconnectReason::HostKey => "host_key",
        _ => "unreachable",
    };
    CommandError::new(
        code,
        detail.unwrap_or_else(|| "the SSH client gave no reason".to_owned()),
    )
}

/// Signs in to the machine an SSH launcher reaches, and leaves again.
pub async fn sign_in(launcher: Launcher) -> Result<(), CommandError> {
    let Launcher::Ssh {
        host,
        user,
        port,
        extra_args,
        client: own,
    } = launcher
    else {
        return Err(CommandError::bad_request("not a host reached over SSH"));
    };
    let args = arguments(&host, user.as_deref(), port, &extra_args)?;
    let program = own
        .filter(|own| !own.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(client)
        .ok_or_else(|| CommandError::unsupported("this machine has no OpenSSH client"))?;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // No console window flashes up for the client.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|error| {
        CommandError::unsupported(format!("the SSH client did not start: {error}"))
    })?;
    let said = Mutex::new(Tail::new());
    let errors = child.stderr.take();
    let running = async {
        if let Some(mut errors) = errors {
            let mut buffer = [0; 1024];
            while let Ok(read) = errors.read(&mut buffer).await
                && read > 0
            {
                said.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(&buffer[..read]);
            }
        }
        child.wait().await
    };
    let ended = tokio::time::timeout(SSH_TRY_WAIT, running)
        .await
        .map_err(|_| {
            failure(
                DisconnectReason::Unreachable,
                Some("no answer in time".into()),
            )
        })?
        .map_err(|error| CommandError::internal(error.to_string()))?;
    if ended.success() {
        return Ok(());
    }
    let status = ended.code().map_or(ExitStatus::Unknown, |code| {
        ExitStatus::Code(code.cast_unsigned())
    });
    let said = said.into_inner().unwrap_or_else(PoisonError::into_inner);
    let ended = classify(status, &said);
    // Its shell ended, so it was reached, whatever the shell made of the word.
    if ended.reason == DisconnectReason::Exited {
        return Ok(());
    }
    Err(failure(ended.reason, ended.detail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_asks_nothing_and_nothing_typed_is_an_option() {
        let key = ["-i".to_owned(), "~/.ssh/key".to_owned()];
        let args = arguments("build", Some("me"), Some(2222), &key).unwrap();
        assert_eq!(args[..2], ["-o", "BatchMode=yes"]);
        let end = ["-i", "~/.ssh/key", "-p", "2222", "--", "me@build", "exit"];
        assert!(args.ends_with(&end.map(str::to_owned)), "{args:?}");
        assert!(arguments("-oProxyCommand=calc", None, None, &[]).is_err());
        assert!(arguments("build", Some("-x"), None, &[]).is_err());
        assert!(arguments("", None, None, &[]).is_err());
    }

    #[tokio::test]
    async fn a_machine_that_is_not_there_is_said_not_to_be() {
        if client().is_none() {
            return;
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let refused = sign_in(Launcher::Ssh {
            host: "127.0.0.1".to_owned(),
            user: Some("nobody".to_owned()),
            port: Some(port),
            extra_args: vec!["-F".to_owned(), "none".to_owned()],
            client: None,
        })
        .await
        .unwrap_err();
        assert_eq!(refused.code, "unreachable", "{refused}");
    }
}
