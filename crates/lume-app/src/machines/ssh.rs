//! The machine an SSH profile signs in to, reached by the system's OpenSSH client.
//!
//! The client runs out of sight and asks nothing: a machine that wants a password, or
//! whose key is not known, is said to want the user to sign in. Only when the user asks
//! for that, by a press in a plugin's view, may it ask, in the window, and the connection
//! it opens is kept for as long as it is used. Nothing that was answered is kept.
//!
//! What its sessions print of their folders is that machine's word only when it names
//! the machine by the name its own shell gave this application: a shell on a further
//! machine that the user went on to names another.

use std::path::PathBuf;

use lume_proto::{DisconnectReason, ExitStatus};
use lume_services::ServiceError;
use lume_services::posix::{CANNOT_RUN, Door, NOT_CONNECTED, SIGN_IN};
use lume_session::ssh::{Tail, Use, classify, client, destination, options};

use crate::tuning::SSH_TRY_CONNECT_SECONDS;

/// How an SSH launcher reaches its machine.
pub(super) struct Reached<'a> {
    pub host: &'a str,
    pub user: Option<&'a str>,
    pub port: Option<u16>,
    pub extra_args: &'a [String],
    pub client: Option<&'a str>,
}

impl Reached<'_> {
    /// The machine's id: how it is reached, and not what the user calls it, so that a
    /// renamed profile keeps what the user agreed to.
    pub fn id(&self) -> String {
        format!("ssh:{}", self.address())
    }

    /// Where it is reached, as the user is shown it.
    pub fn address(&self) -> String {
        let at = destination(self.host, self.user);
        match self.port {
            Some(port) => format!("{at}:{port}"),
            None => at,
        }
    }

    /// How its shell is reached. `asks` is what the client asks the user with, where it
    /// may; without it, it asks nothing.
    pub fn door(
        &self,
        label: &str,
        asks: Option<Vec<(String, String)>>,
    ) -> Result<Door, ServiceError> {
        let program = self
            .client
            .filter(|own| !own.is_empty())
            .map(PathBuf::from)
            .or_else(client)
            .ok_or_else(|| {
                ServiceError::unavailable("this machine has no OpenSSH client").because(CANNOT_RUN)
            })?;
        let mut args = options(
            self.host,
            self.user,
            self.port,
            self.extra_args,
            Use {
                asks: asks.is_some(),
                connect_seconds: SSH_TRY_CONNECT_SECONDS,
                alive: true,
                input: true,
            },
        )
        .map_err(|error| ServiceError::invalid(error.message))?;
        args.extend(["--".to_owned(), destination(self.host, self.user)]);
        Ok(Door {
            program,
            args,
            env: asks.unwrap_or_default(),
            one_line: true,
            label: label.to_owned(),
            unreached: Some(unreached),
        })
    }
}

/// What a client that did not reach a shell over there failed for. One that ended by
/// itself with the other side's own status reached it, and that side has no `sh`: that
/// is for the caller to say.
fn unreached(code: Option<i32>, said: &[u8]) -> Option<&'static str> {
    let status = code.map_or(ExitStatus::Unknown, |code| {
        ExitStatus::Code(code.cast_unsigned())
    });
    let mut tail = Tail::new();
    tail.push(said);
    match classify(status, &tail).reason {
        DisconnectReason::AuthFailed | DisconnectReason::HostKey => Some(SIGN_IN),
        DisconnectReason::Exited => None,
        _ => Some(NOT_CONNECTED),
    }
}

/// The longest name a machine is taken to give itself.
const NAME_MAX: usize = 255;

/// Whether `reported`, the machine a shell named in its report, is the one whose shell
/// called itself `name`. Short names are compared, without case: a shell may give either.
/// Nothing at all and `localhost` name no machine in particular, so they are none.
pub(super) fn same_machine(reported: &str, name: &str) -> bool {
    let short = |name: &str| name.split('.').next().unwrap_or_default().to_lowercase();
    let (reported, own) = (short(reported), short(name));
    let named = |name: &str| {
        !name.is_empty()
            && name.len() <= NAME_MAX
            && name != "localhost"
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    named(&own) && named(&reported) && reported == own
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reached<'a>(user: Option<&'a str>, port: Option<u16>, extra: &'a [String]) -> Reached<'a> {
        Reached {
            host: "devbox",
            user,
            port,
            extra_args: extra,
            client: Some("ssh-here"),
        }
    }

    #[test]
    fn a_machine_is_known_by_how_it_is_reached() {
        assert_eq!(reached(None, None, &[]).id(), "ssh:devbox");
        assert_eq!(
            reached(Some("me"), Some(2222), &[]).id(),
            "ssh:me@devbox:2222"
        );
        assert_eq!(reached(Some("me"), None, &[]).address(), "me@devbox");
    }

    #[test]
    fn the_client_asks_nothing_unless_it_may_and_is_handed_one_line() {
        let extra = ["-i".to_owned(), "key".to_owned()];
        let door = reached(Some("me"), Some(2), &extra)
            .door("Dev", None)
            .unwrap();
        assert_eq!(door.program, PathBuf::from("ssh-here"));
        assert!(door.one_line && door.env.is_empty());
        assert_eq!(door.args[..2], ["-o", "BatchMode=yes"]);
        assert!(!door.args.contains(&"-n".to_owned()), "{:?}", door.args);
        assert!(
            door.args
                .ends_with(&["-i", "key", "-p", "2", "--", "me@devbox"].map(str::to_owned))
        );
        let env = vec![("SSH_ASKPASS".to_owned(), "lume".to_owned())];
        let asking = reached(Some("me"), Some(2), &extra)
            .door("Dev", Some(env.clone()))
            .unwrap();
        assert!(!asking.args.contains(&"BatchMode=yes".to_owned()));
        assert_eq!(asking.env, env);
        let bad = Reached {
            host: "-oProxyCommand=calc",
            ..reached(None, None, &[])
        };
        assert!(bad.door("x", None).is_err());
    }

    #[test]
    fn what_the_client_says_of_its_end_is_what_it_failed_for() {
        let cases: [(Option<i32>, &str, Option<&str>); 6] = [
            (
                Some(255),
                "me@devbox: Permission denied (publickey,password).",
                Some(SIGN_IN),
            ),
            (Some(255), "Host key verification failed.", Some(SIGN_IN)),
            (
                Some(255),
                "ssh: connect to host devbox port 22: Connection refused",
                Some(NOT_CONNECTED),
            ),
            (Some(255), "", Some(NOT_CONNECTED)),
            // The shell over there ended with its own status: it was reached.
            (Some(127), "sh: not found", None),
            (
                Some(1),
                "'sh' is not recognized as an internal or external command",
                None,
            ),
        ];
        for (code, said, why) in cases {
            assert_eq!(unreached(code, said.as_bytes()), why, "{said}");
        }
    }

    #[test]
    fn a_report_names_the_machine_only_by_the_name_it_gave_itself() {
        assert!(same_machine("devbox", "devbox"));
        assert!(same_machine("DevBox.lan", "devbox"));
        assert!(same_machine("devbox", "devbox.example.org"));
        for (reported, name) in [
            ("other", "devbox"),
            ("", "devbox"),
            ("localhost", "localhost"),
            ("devbox", ""),
            ("dev box", "dev box"),
            ("dev\u{0}box", "dev\u{0}box"),
            (&"x".repeat(300), &"x".repeat(300)),
        ] {
            assert!(!same_machine(reported, name), "{reported:?} {name:?}");
        }
    }
}
