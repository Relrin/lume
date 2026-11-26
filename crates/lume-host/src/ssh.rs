//! The system's OpenSSH client, for whoever else has to run it: where it is, which
//! words it may be handed, and what its end says.
//!
//! A session started over SSH runs the client in its terminal. Something that runs it
//! out of sight, to carry a connection of its own, finds and reads it the same way.

use std::path::PathBuf;

use lume_proto::ProtoError;

pub use crate::ssh_exit::{Tail, classify};

/// The OpenSSH client of this machine: the system's own where there is one, else the
/// first on `PATH`. Nothing when the machine has none.
pub fn client() -> Option<PathBuf> {
    crate::launch::ssh_client_here()
}

/// Refuses a word that names a machine or an account to the client if the client would
/// not take it as a name: nothing at all, an option, or something with control
/// characters in it. `what` says what the word is, for the refusal.
pub fn check_word(what: &str, word: &str) -> Result<(), ProtoError> {
    crate::launch::check_word(what, word)
}

/// How a client that runs out of sight is to behave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Use {
    /// Whether it may ask what it has to know. One that may not is told to ask nothing.
    pub asks: bool,
    /// How long it may take to reach the other machine.
    pub connect_seconds: u32,
    /// Whether it asks the other side now and then whether it is still there, so that a
    /// connection that is gone is noticed.
    pub alive: bool,
    /// Whether what this side writes is to reach the other side. Without it, its input
    /// is nothing at all.
    pub input: bool,
}

/// The arguments a client that runs out of sight begins with, before what follows `--`.
/// What is fixed comes first: the client keeps the first value an option is given, so
/// nothing of the user's own `extra_args` undoes it. A connection of the user's own that
/// others share is never used: it is not this one's to end.
pub fn options(
    host: &str,
    user: Option<&str>,
    port: Option<u16>,
    extra_args: &[String],
    how: Use,
) -> Result<Vec<String>, ProtoError> {
    check_word("host", host)?;
    if let Some(user) = user {
        check_word("user's name", user)?;
    }
    let mut args = Vec::new();
    if !how.asks {
        args.extend(["-o".to_owned(), "BatchMode=yes".to_owned()]);
    }
    let mut fixed = vec![
        format!("ConnectTimeout={}", how.connect_seconds),
        "ControlPath=none".to_owned(),
    ];
    if how.alive {
        fixed.extend(["ServerAliveInterval=15", "ServerAliveCountMax=3"].map(str::to_owned));
    }
    for option in fixed {
        args.extend(["-o".to_owned(), option]);
    }
    args.push("-T".to_owned());
    if !how.input {
        args.push("-n".to_owned());
    }
    args.extend(extra_args.iter().cloned());
    if let Some(port) = port {
        args.extend(["-p".to_owned(), port.to_string()]);
    }
    Ok(args)
}

/// The account and machine, as the client takes them after `--`.
pub fn destination(host: &str, user: Option<&str>) -> String {
    match user {
        Some(user) => format!("{user}@{host}"),
        None => host.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn how(asks: bool, alive: bool, input: bool) -> Use {
        Use {
            asks,
            connect_seconds: 10,
            alive,
            input,
        }
    }

    #[test]
    fn what_is_fixed_comes_before_anything_of_the_users() {
        let own = ["-o".to_owned(), "ControlPath=~/.ssh/%C".to_owned()];
        let args = options("box", Some("me"), Some(2222), &own, how(false, true, false)).unwrap();
        assert_eq!(
            args,
            [
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "ControlPath=none",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=3",
                "-T",
                "-n",
                "-o",
                "ControlPath=~/.ssh/%C",
                "-p",
                "2222",
            ]
        );
        let asking = options("box", None, None, &[], how(true, false, true)).unwrap();
        assert_eq!(
            asking,
            ["-o", "ConnectTimeout=10", "-o", "ControlPath=none", "-T"]
        );
        assert_eq!(destination("box", Some("me")), "me@box");
        assert_eq!(destination("box", None), "box");
    }

    #[test]
    fn nothing_typed_is_an_option() {
        let how = how(false, false, false);
        assert!(options("-oProxyCommand=calc", None, None, &[], how).is_err());
        assert!(options("box", Some("-x"), None, &[], how).is_err());
        assert!(options("", None, None, &[], how).is_err());
        assert!(options("bo\nx", None, None, &[], how).is_err());
    }
}
