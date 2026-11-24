//! How near the machine a profile reaches over SSH is: how long a TCP connection to its
//! SSH port takes to open.
//!
//! The machine is found the way the user's OpenSSH client would find it: `ssh -G` says
//! which host name and port its configuration gives the profile's host, so an alias from
//! `~/.ssh/config` is measured where it leads. A host reached through a jump host or a
//! proxy command is not measured, since a connection from here says nothing of that
//! route. Nothing is sent over the connection: it is opened, timed and closed.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;

use lume_session::Launcher;
use lume_session::ssh::check_word;
use serde::Serialize;
use tokio::net::{TcpStream, lookup_host};

use crate::tuning::{HOST_PROBE_TIMEOUT, SSH_CONFIG_POLL, SSH_CONFIG_WAIT};

/// The port SSH listens on unless something says otherwise.
const SSH_PORT: u16 = 22;

/// What measuring a host found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Probe {
    /// How long the connection took to open, in milliseconds; none when it did not open.
    pub ms: Option<u64>,
    /// Why there is no time, in words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Whether the machine is reached through another, so that a connection from here
    /// would say nothing of it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub relayed: bool,
}

impl Probe {
    pub fn without(note: impl Into<String>) -> Self {
        Self {
            ms: None,
            note: Some(note.into()),
            relayed: false,
        }
    }

    /// The machine answered within so many milliseconds.
    pub fn within(ms: u64) -> Self {
        Self {
            ms: Some(ms),
            note: None,
            relayed: false,
        }
    }
}

/// Where a connection goes, as the client's configuration says.
#[derive(Debug, PartialEq, Eq)]
struct Target {
    host: String,
    port: u16,
    /// Whether the client goes through another machine to get there.
    relayed: bool,
}

/// Reads what `ssh -G` printed: the host name, the port, and whether a jump host or a
/// proxy command stands between. What it does not say is taken from `asked`.
fn read_config(printed: &str, asked: &Target) -> Target {
    let mut target = Target {
        host: asked.host.clone(),
        port: asked.port,
        relayed: false,
    };
    for line in printed.lines() {
        let Some((key, value)) = line.trim().split_once(' ') else {
            continue;
        };
        let value = value.trim();
        match key.to_ascii_lowercase().as_str() {
            "hostname" if !value.is_empty() => target.host = value.to_owned(),
            "port" => {
                if let Ok(port) = value.parse() {
                    target.port = port;
                }
            }
            "proxyjump" | "proxycommand" if !value.eq_ignore_ascii_case("none") => {
                target.relayed = true;
            }
            _ => {}
        }
    }
    target
}

/// The OpenSSH client to ask: the profile's own, else the system's, else the first on
/// `PATH`. Only absolute folders of `PATH` are looked in.
fn client_of(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(client) = explicit.filter(|client| !client.is_empty()) {
        return Some(PathBuf::from(client));
    }
    let file = if cfg!(windows) { "ssh.exe" } else { "ssh" };
    if cfg!(windows) {
        let inbox = std::env::var_os("SystemRoot")
            .map(|root| {
                PathBuf::from(root)
                    .join("System32")
                    .join("OpenSSH")
                    .join(file)
            })
            .filter(|client| client.is_file());
        if inbox.is_some() {
            return inbox;
        }
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(file))
        .find(|candidate| candidate.is_file())
}

/// Asks the client where it would connect for `asked`, given the profile's own
/// arguments. A client that cannot be asked, or does not answer in time, leaves the
/// host and port as the profile names them.
fn where_to(asked: Target, extra_args: &[String], client: Option<&str>) -> Target {
    let Some(client) = client_of(client) else {
        return asked;
    };
    let mut command = Command::new(client);
    command.arg("-G").args(extra_args);
    if asked.port != SSH_PORT {
        command.arg("-p").arg(asked.port.to_string());
    }
    command
        .arg(&asked.host)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashes up for the client.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let Ok(mut child) = command.spawn() else {
        return asked;
    };
    let deadline = Instant::now() + SSH_CONFIG_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(SSH_CONFIG_POLL),
            _ => {
                // It did not answer: it is not left running on the application's account.
                let _ = child.kill();
                let _ = child.wait();
                return asked;
            }
        }
    }
    let mut printed = String::new();
    if let Some(mut out) = child.stdout.take() {
        use std::io::Read as _;
        let _ = out.read_to_string(&mut printed);
    }
    read_config(&printed, &asked)
}

/// Measures the machine an SSH launcher reaches.
pub async fn probe(launcher: Launcher) -> Probe {
    let Launcher::Ssh {
        host,
        port,
        extra_args,
        client,
        ..
    } = launcher
    else {
        return Probe::without("not a host reached over SSH");
    };
    // The client is asked about it, and would read a word that begins with a hyphen as
    // an option.
    if let Err(error) = check_word("host", &host) {
        return Probe::without(error.message);
    }
    let asked = Target {
        host,
        port: port.unwrap_or(SSH_PORT),
        relayed: false,
    };
    let target =
        match tokio::task::spawn_blocking(move || where_to(asked, &extra_args, client.as_deref()))
            .await
        {
            Ok(target) => target,
            Err(_) => return Probe::without("the SSH configuration could not be read"),
        };
    if target.relayed {
        return Probe {
            relayed: true,
            ..Probe::without("reached through another machine")
        };
    }
    let address = match tokio::time::timeout(
        HOST_PROBE_TIMEOUT,
        lookup_host((target.host.as_str(), target.port)),
    )
    .await
    {
        Ok(Ok(mut found)) => match found.next() {
            Some(address) => address,
            None => return Probe::without("the host name was not found"),
        },
        Ok(Err(_)) => return Probe::without("the host name was not found"),
        Err(_) => return Probe::without("the host name took too long to look up"),
    };
    let started = Instant::now();
    match tokio::time::timeout(HOST_PROBE_TIMEOUT, TcpStream::connect(address)).await {
        Ok(Ok(_)) => {
            Probe::within(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX))
        }
        Ok(Err(_)) => Probe::without("the host did not take the connection"),
        Err(_) => Probe::without("the host did not answer in time"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(host: &str, port: u16) -> Target {
        Target {
            host: host.to_owned(),
            port,
            relayed: false,
        }
    }

    #[test]
    fn the_client_configuration_names_where_an_alias_leads() {
        let printed = "user mira\nhostname 10.0.4.21\nport 2222\nproxyjump none\n";
        assert_eq!(
            read_config(printed, &asked("build-box", 22)),
            Target {
                host: "10.0.4.21".to_owned(),
                port: 2222,
                relayed: false
            }
        );
    }

    #[test]
    fn a_jump_host_or_a_proxy_command_is_seen() {
        assert!(read_config("hostname inner\nproxyjump bastion\n", &asked("inner", 22)).relayed);
        assert!(read_config("proxycommand nc %h %p\n", &asked("inner", 22)).relayed);
        assert!(!read_config("proxycommand none\n", &asked("inner", 22)).relayed);
    }

    #[test]
    fn nothing_printed_keeps_what_the_profile_names() {
        assert_eq!(
            read_config("", &asked("devbox", 2200)),
            asked("devbox", 2200)
        );
    }

    #[tokio::test]
    async fn a_shell_here_is_not_measured() {
        let probe = probe(Launcher::Shell {
            program: String::new(),
            args: Vec::new(),
            login: false,
        })
        .await;
        assert_eq!(probe.ms, None);
    }

    #[tokio::test]
    async fn a_word_the_client_would_read_as_an_option_is_not_asked_about() {
        let probe = probe(Launcher::Ssh {
            host: "-oProxyCommand=calc".to_owned(),
            user: None,
            port: None,
            extra_args: Vec::new(),
            client: None,
        })
        .await;
        assert!(probe.ms.is_none() && probe.note.is_some(), "{probe:?}");
    }

    #[tokio::test]
    async fn an_open_port_here_is_measured() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let probe = probe(Launcher::Ssh {
            host: "127.0.0.1".to_owned(),
            user: None,
            port: Some(port),
            extra_args: Vec::new(),
            // A client that is not there leaves the host and port as named.
            client: Some("Z:\\no\\such\\ssh.exe".to_owned()),
        })
        .await;
        assert!(probe.ms.is_some(), "{probe:?}");
    }
}
