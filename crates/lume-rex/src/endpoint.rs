//! Where a Rex server is reached.

use std::fmt;
use std::net::{IpAddr, Ipv6Addr};
use std::path::PathBuf;

use crate::error::RexError;

/// The port an `https://` address means when it names none.
pub const HTTPS_PORT: u16 = 443;
/// The port an `http://` address means when it names none.
pub const HTTP_PORT: u16 = 80;
/// The port a Rex server is usually told to listen on.
pub const REX_PORT: u16 = 7297;

/// The longest name a host has.
const HOST_MAX_BYTES: usize = 253;
/// The longest name of an account on a host that is taken.
const USER_MAX_BYTES: usize = 64;
/// The longest path of a socket on another machine that is taken.
const SOCKET_MAX_BYTES: usize = 1024;

/// Where a Rex server is reached.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Endpoint {
    /// The user's own server on this machine, wherever it listens: its named pipe on
    /// Windows, its socket in the user's runtime directory elsewhere.
    Local,
    /// A named pipe on this machine, by its whole name, such as `\\.\pipe\rex-S-1-5-…`.
    Pipe(String),
    /// A Unix socket on this machine.
    Unix(PathBuf),
    /// A server behind TLS. The host is a name or an address, lower case, an IPv6
    /// address without its brackets.
    Https { host: String, port: u16 },
    /// A server reached with nothing between it and whoever is on the network.
    Http { host: String, port: u16 },
    /// A server listening on its machine's tailnet address: nothing of its own between
    /// the two ends, because the tailnet encrypts what passes.
    Tailnet { host: String, port: u16 },
    /// A host with nothing said of how it is reached, as a person writes one. It is
    /// tried behind TLS first, on the usual ports unless one is named.
    Host { host: String, port: Option<u16> },
    /// A server reached through the system's SSH client, as the account that logs in.
    Ssh {
        /// The account to log in as; the SSH client's own choice when none.
        user: Option<String>,
        host: String,
        /// The port of the SSH server; the SSH client's own choice when none.
        port: Option<u16>,
        /// The server's socket on that machine; the account's own server when none.
        socket: Option<String>,
    },
}

/// A host as it stands in an address: a name of letters, digits, `-`, `.` and `_`, or
/// an IP address. Nothing else is taken, so that a host can never be read as an option
/// of a program it is handed to, or as more than a host in a header.
fn host(text: &str) -> Option<String> {
    if let Ok(address) = text.parse::<IpAddr>() {
        return Some(address.to_string());
    }
    let named = !text.is_empty()
        && text.len() <= HOST_MAX_BYTES
        && !text.starts_with(['-', '.'])
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'));
    named.then(|| text.to_ascii_lowercase())
}

/// A host and perhaps a port, as they stand together: `name`, `name:1`, `[::1]`,
/// `[::1]:1`, and an IPv6 address alone.
fn host_port(text: &str) -> Option<(String, Option<u16>)> {
    if let Some(rest) = text.strip_prefix('[') {
        let (address, after) = rest.split_once(']')?;
        let address = address.parse::<Ipv6Addr>().ok()?.to_string();
        return match after.strip_prefix(':') {
            Some(port) => Some((address, Some(port_of(port)?))),
            None => after.is_empty().then_some((address, None)),
        };
    }
    if let Ok(address) = text.parse::<Ipv6Addr>() {
        return Some((address.to_string(), None));
    }
    match text.rsplit_once(':') {
        Some((name, port)) => Some((host(name)?, Some(port_of(port)?))),
        None => Some((host(text)?, None)),
    }
}

/// A port as it is written: digits only, and not zero.
fn port_of(text: &str) -> Option<u16> {
    let digits = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    text.parse().ok().filter(|port| digits && *port != 0)
}

/// An account as it stands before a host.
fn user_of(text: &str) -> Option<String> {
    let plain = !text.is_empty()
        && text.len() <= USER_MAX_BYTES
        && !text.starts_with('-')
        && text
            .chars()
            .all(|char| !char.is_control() && !char.is_whitespace() && !"@:/".contains(char));
    plain.then(|| text.to_owned())
}

/// A host as it stands in an address or a header: an IPv6 address in brackets.
fn bracketed(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

/// A host and a port as a server is named in a request: the port left out where it is
/// the one the kind of address means.
pub(crate) fn authority(host: &str, port: u16, usual: u16) -> String {
    if port == usual {
        bracketed(host)
    } else {
        format!("{}:{port}", bracketed(host))
    }
}

impl Endpoint {
    /// Reads an address as a person or a Rex server writes one.
    ///
    /// - `local` or nothing: the user's own server on this machine.
    /// - `unix://PATH`, or a path alone: a Unix socket.
    /// - `npipe:////./pipe/NAME`, or a name that begins `\\.\pipe\`: a named pipe.
    /// - `https://HOST[:PORT]` and `http://HOST[:PORT]`.
    /// - `tailnet://HOST:PORT`: a server on its machine's tailnet address.
    /// - `ssh://[USER@]HOST[:PORT][/SOCKET]`.
    /// - `HOST[:PORT]` alone.
    pub fn parse(address: &str) -> Result<Self, RexError> {
        let address = address.trim();
        let bad = || RexError::BadEndpoint(address.to_owned());
        if address.is_empty() || address.eq_ignore_ascii_case("local") {
            return Ok(Endpoint::Local);
        }
        if let Some(path) = address.strip_prefix("unix://") {
            return match path {
                "" => Err(bad()),
                path => Ok(Endpoint::Unix(PathBuf::from(path))),
            };
        }
        if let Some(rest) = address.strip_prefix("npipe:") {
            // A server writes `npipe:////./pipe/NAME`; `npipe://./pipe/NAME` is the same.
            let name = rest
                .trim_start_matches('/')
                .strip_prefix("./pipe/")
                .filter(|name| !name.is_empty() && !name.contains(['/', '\\']))
                .ok_or_else(bad)?;
            return Ok(Endpoint::Pipe(format!(r"\\.\pipe\{name}")));
        }
        if let Some(name) = address.strip_prefix(r"\\.\pipe\") {
            return match name {
                "" => Err(bad()),
                _ => Ok(Endpoint::Pipe(address.to_owned())),
            };
        }
        // An account before the host of a server asks a login server for that account.
        // Nothing a server says of itself tells how the account is asked for.
        let account = || RexError::Unsupported {
            endpoint: address.to_owned(),
            why: "an account cannot be chosen in the address of a server; only `ssh://` \
                  addresses take one"
                .to_owned(),
        };
        if let Some((scheme, rest)) = address.split_once("://") {
            let scheme = scheme.to_ascii_lowercase();
            if scheme == "ssh" {
                return Self::parse_ssh(rest).ok_or_else(bad);
            }
            if !matches!(scheme.as_str(), "https" | "http" | "tailnet") {
                return Err(bad());
            }
            let rest = rest.strip_suffix('/').unwrap_or(rest);
            if rest.contains('@') {
                return Err(account());
            }
            let (host, port) = host_port(rest).ok_or_else(bad)?;
            return match scheme.as_str() {
                "https" => Ok(Endpoint::Https {
                    host,
                    port: port.unwrap_or(HTTPS_PORT),
                }),
                "http" => Ok(Endpoint::Http {
                    host,
                    port: port.unwrap_or(HTTP_PORT),
                }),
                // A tailnet listener has no usual port: a server is told one.
                _ => Ok(Endpoint::Tailnet {
                    host,
                    port: port.ok_or_else(bad)?,
                }),
            };
        }
        if address.starts_with('/') {
            return Ok(Endpoint::Unix(PathBuf::from(address)));
        }
        if address.contains('@') {
            return Err(account());
        }
        let (host, port) = host_port(address).ok_or_else(bad)?;
        Ok(Endpoint::Host { host, port })
    }

    /// What follows `ssh://`.
    fn parse_ssh(rest: &str) -> Option<Self> {
        let (place, socket) = match rest.find('/') {
            Some(at) => (&rest[..at], &rest[at..]),
            None => (rest, ""),
        };
        let socket = match socket {
            "" | "/" => None,
            path => {
                let plain = path.len() <= SOCKET_MAX_BYTES && !path.chars().any(char::is_control);
                Some(plain.then(|| path.to_owned())?)
            }
        };
        let (user, place) = match place.rsplit_once('@') {
            Some((user, place)) => (Some(user_of(user)?), place),
            None => (None, place),
        };
        let (host, port) = host_port(place)?;
        Some(Endpoint::Ssh {
            user,
            host,
            port,
            socket,
        })
    }

    /// Whether the server is on this machine, reached without a network.
    pub fn is_on_this_machine(&self) -> bool {
        matches!(
            self,
            Endpoint::Local | Endpoint::Pipe(_) | Endpoint::Unix(_)
        )
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Endpoint::Local => f.write_str("local"),
            Endpoint::Pipe(name) => {
                let name = name.strip_prefix(r"\\.\pipe\").unwrap_or(name);
                write!(f, "npipe:////./pipe/{name}")
            }
            Endpoint::Unix(path) => write!(f, "unix://{}", path.display()),
            Endpoint::Https { host, port } => {
                write!(f, "https://{}", authority(host, *port, HTTPS_PORT))
            }
            Endpoint::Http { host, port } => {
                write!(f, "http://{}", authority(host, *port, HTTP_PORT))
            }
            Endpoint::Tailnet { host, port } => write!(f, "tailnet://{}:{port}", bracketed(host)),
            Endpoint::Host { host, port } => match port {
                Some(port) => write!(f, "{}:{port}", bracketed(host)),
                // An IPv6 address alone is written as it is read.
                None => f.write_str(host),
            },
            Endpoint::Ssh {
                user,
                host,
                port,
                socket,
            } => {
                f.write_str("ssh://")?;
                if let Some(user) = user {
                    write!(f, "{user}@")?;
                }
                f.write_str(&bracketed(host))?;
                if let Some(port) = port {
                    write!(f, ":{port}")?;
                }
                f.write_str(socket.as_deref().unwrap_or(""))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_read_as_a_server_and_a_person_write_them() {
        assert_eq!(Endpoint::parse("").unwrap(), Endpoint::Local);
        assert_eq!(Endpoint::parse(" Local ").unwrap(), Endpoint::Local);
        assert_eq!(
            Endpoint::parse("unix:///run/user/1000/rex/server.sock").unwrap(),
            Endpoint::Unix(PathBuf::from("/run/user/1000/rex/server.sock"))
        );
        assert_eq!(
            Endpoint::parse("/tmp/rex.sock").unwrap(),
            Endpoint::Unix(PathBuf::from("/tmp/rex.sock"))
        );
        let pipe = Endpoint::Pipe(r"\\.\pipe\rex-S-1-5-21-1".to_owned());
        assert_eq!(
            Endpoint::parse("npipe:////./pipe/rex-S-1-5-21-1").unwrap(),
            pipe
        );
        assert_eq!(
            Endpoint::parse("npipe://./pipe/rex-S-1-5-21-1").unwrap(),
            pipe
        );
        assert_eq!(Endpoint::parse(r"\\.\pipe\rex-S-1-5-21-1").unwrap(), pipe);
    }

    #[test]
    fn addresses_of_servers_elsewhere_are_read() {
        let read = |address: &str| Endpoint::parse(address).unwrap();
        assert_eq!(
            read("https://Build.Example.ts.net"),
            Endpoint::Https {
                host: "build.example.ts.net".to_owned(),
                port: 443
            }
        );
        assert_eq!(
            read("HTTPS://build.example.ts.net:7297/"),
            Endpoint::Https {
                host: "build.example.ts.net".to_owned(),
                port: 7297
            }
        );
        assert_eq!(
            read("http://10.0.0.5:7297"),
            Endpoint::Http {
                host: "10.0.0.5".to_owned(),
                port: 7297
            }
        );
        assert_eq!(
            read("http://[::1]:7297"),
            Endpoint::Http {
                host: "::1".to_owned(),
                port: 7297
            }
        );
        assert_eq!(
            read("http://localhost"),
            Endpoint::Http {
                host: "localhost".to_owned(),
                port: 80
            }
        );
        assert_eq!(
            read("tailnet://build.example.ts.net:7297"),
            Endpoint::Tailnet {
                host: "build.example.ts.net".to_owned(),
                port: 7297
            }
        );
        assert_eq!(
            read("studio.tail1234.ts.net"),
            Endpoint::Host {
                host: "studio.tail1234.ts.net".to_owned(),
                port: None
            }
        );
        assert_eq!(
            read("192.168.1.20:7297"),
            Endpoint::Host {
                host: "192.168.1.20".to_owned(),
                port: Some(7297)
            }
        );
        assert_eq!(
            read("fd7a:115c:a1e0::1"),
            Endpoint::Host {
                host: "fd7a:115c:a1e0::1".to_owned(),
                port: None
            }
        );
        assert_eq!(
            read("ssh://alice@build.example.com"),
            Endpoint::Ssh {
                user: Some("alice".to_owned()),
                host: "build.example.com".to_owned(),
                port: None,
                socket: None
            }
        );
        assert_eq!(
            read("ssh://build:2222/run/rex/server.sock"),
            Endpoint::Ssh {
                user: None,
                host: "build".to_owned(),
                port: Some(2222),
                socket: Some("/run/rex/server.sock".to_owned())
            }
        );
    }

    #[test]
    fn an_address_is_written_as_it_is_read() {
        for address in [
            "local",
            "unix:///run/rex/server.sock",
            "npipe:////./pipe/rex-S-1-5-21-1",
            "https://build.example.ts.net",
            "https://build.example.ts.net:7297",
            "http://10.0.0.5:7297",
            "http://localhost",
            "http://[::1]:7297",
            "tailnet://build.example.ts.net:7297",
            "tailnet://[fd7a:115c:a1e0::1]:7297",
            "studio.tail1234.ts.net",
            "192.168.1.20:7297",
            "[::1]:7297",
            "::1",
            "ssh://build.example.com",
            "ssh://alice@build.example.com:2222",
            "ssh://alice@[::1]/run/rex/server.sock",
        ] {
            let endpoint = Endpoint::parse(address).unwrap();
            assert_eq!(endpoint.to_string(), address);
            assert_eq!(Endpoint::parse(&endpoint.to_string()).unwrap(), endpoint);
        }
    }

    #[test]
    fn only_a_server_on_this_machine_is_said_to_be() {
        for (address, here) in [
            ("local", true),
            ("unix:///run/rex/server.sock", true),
            ("npipe://./pipe/rex", true),
            ("http://127.0.0.1:7297", false),
            ("https://build.example.ts.net", false),
            ("ssh://build", false),
            ("build", false),
        ] {
            let endpoint = Endpoint::parse(address).unwrap();
            assert_eq!(endpoint.is_on_this_machine(), here, "{address}");
        }
    }

    #[test]
    fn what_is_not_an_address_this_client_reaches_is_refused() {
        for address in [
            "unix://",
            "npipe://./pipe/",
            "npipe://elsewhere/pipe/rex",
            "npipe://./pipe/a/b",
            r"\\.\pipe\",
            "ftp://rex.example",
            "https://",
            "https://rex.example/api",
            "https://rex.example?x=1",
            "https://rex.example:0",
            "https://rex.example:65536",
            "https://rex.example:+80",
            "https://rex example",
            "https://-oProxyCommand=x",
            "http://[::1",
            "http://[rex]:1",
            "tailnet://build.example.ts.net",
            "ssh://",
            "ssh://-oProxyCommand=x",
            "ssh://-l@host",
            "ssh://a b@host",
            "ssh://host:port",
            "ssh://host/run/\u{7}rex.sock",
            "-V",
            "rex.example/api",
            r"C:\rex\server.sock",
            "caf\u{e9}.example",
        ] {
            let refused = Endpoint::parse(address).unwrap_err();
            assert_eq!(refused.code(), "bad_request", "{address}");
        }
    }

    #[test]
    fn an_account_in_the_address_of_a_server_is_said_to_be_beyond_this_client() {
        for address in [
            "https://root@build.example.ts.net",
            "http://alice@10.0.0.5:7297",
            "root@build.example.ts.net",
        ] {
            let refused = Endpoint::parse(address).unwrap_err();
            assert_eq!(refused.code(), "unsupported", "{address}");
        }
    }

    #[test]
    fn a_server_is_named_in_a_request_as_its_address_names_it() {
        assert_eq!(authority("rex.example", 443, HTTPS_PORT), "rex.example");
        assert_eq!(
            authority("rex.example", 7297, HTTPS_PORT),
            "rex.example:7297"
        );
        assert_eq!(authority("::1", 7297, HTTP_PORT), "[::1]:7297");
        assert_eq!(authority("::1", 80, HTTP_PORT), "[::1]");
    }
}
