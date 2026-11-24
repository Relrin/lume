//! Whether a machine can be signed in to over SSH, found out before a profile for it is
//! written.
//!
//! The client runs out of sight. What it has to know is asked in the window the try was
//! made in. Where nothing can be asked, an account that needs a password, or a machine
//! whose key is not known, is said to, and is settled in a terminal.

use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};

use lume_proto::{DisconnectReason, ExitStatus};
use lume_session::Launcher;
use lume_session::ssh::{Tail, check_word, classify, client};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::error::CommandError;
use crate::rex::{Attempt, GIVEN_UP};
use crate::tuning::{SSH_TRY_CONNECT_SECONDS, SSH_TRY_WAIT};

/// What the other machine's shell is told to do: to end.
const ASK: &str = "exit";

/// The arguments the client is run with. The destination follows `--`, so nothing typed
/// is read as an option. One that `asks` is not told to ask nothing.
fn arguments(
    host: &str,
    user: Option<&str>,
    port: Option<u16>,
    extra_args: &[String],
    asks: bool,
) -> Result<Vec<String>, CommandError> {
    let word = |what: &str, word: &str| {
        check_word(what, word).map_err(|error| CommandError::bad_request(error.message))
    };
    word("host", host)?;
    if let Some(user) = user {
        word("user's name", user)?;
    }
    let mut args = Vec::new();
    if !asks {
        args.extend(["-o".to_owned(), "BatchMode=yes".to_owned()]);
    }
    for option in [
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

/// Signs in to the machine an SSH launcher reaches, and leaves again. The client asks
/// what it has to know through `asks`, if it can.
pub async fn sign_in(launcher: Launcher, asks: Option<Arc<Attempt>>) -> Result<(), CommandError> {
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
    // Before the client, and so let go of after it: a client that is told that nobody
    // answers tries an empty password. It is ended first.
    let run = asks.as_ref().and_then(|asks| asks.lets_ask());
    let args = arguments(&host, user.as_deref(), port, &extra_args, run.is_some())?;
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
    if let Some(run) = &run {
        command.envs(run.env.iter().map(|(name, value)| (name, value)));
    }
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
    let late = || {
        failure(
            DisconnectReason::Unreachable,
            Some("no answer in time".into()),
        )
    };
    let ended = match &asks {
        None => tokio::time::timeout(SSH_TRY_WAIT, running)
            .await
            .map_err(|_| late()),
        Some(asks) => {
            let clock = asks.clock();
            tokio::select! {
                biased;
                () = asks.ended() => Err(CommandError::new(
                    GIVEN_UP,
                    format!("signing in to {host} was given up"),
                )),
                ended = clock.within(SSH_TRY_WAIT, running) => ended.ok_or_else(late),
            }
        }
    };
    let ended = match ended {
        Ok(ended) => ended.map_err(|error| CommandError::internal(error.to_string()))?,
        Err(error) => {
            let _ = child.start_kill();
            return Err(error);
        }
    };
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
        let args = arguments("build", Some("me"), Some(2222), &key, false).unwrap();
        assert_eq!(args[..2], ["-o", "BatchMode=yes"]);
        let asking = arguments("build", Some("me"), Some(2222), &key, true).unwrap();
        assert_eq!(asking[..], args[2..]);
        let end = ["-i", "~/.ssh/key", "-p", "2222", "--", "me@build", "exit"];
        assert!(args.ends_with(&end.map(str::to_owned)), "{args:?}");
        assert!(arguments("-oProxyCommand=calc", None, None, &[], false).is_err());
        assert!(arguments("build", Some("-x"), None, &[], false).is_err());
        assert!(arguments("", None, None, &[], false).is_err());
    }

    #[tokio::test]
    async fn a_machine_that_is_not_there_is_said_not_to_be() {
        if client().is_none() {
            return;
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let launcher = Launcher::Ssh {
            host: "127.0.0.1".to_owned(),
            user: Some("nobody".to_owned()),
            port: Some(port),
            extra_args: vec!["-F".to_owned(), "none".to_owned()],
            client: None,
        };
        let refused = sign_in(launcher, None).await.unwrap_err();
        assert_eq!(refused.code, "unreachable", "{refused}");
    }
}
