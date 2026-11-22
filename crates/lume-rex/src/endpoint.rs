//! Where a Rex server is reached.

use std::fmt;
use std::path::PathBuf;

use crate::error::RexError;

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
}

impl Endpoint {
    /// Reads an address as a person or a Rex server writes one: `local` or nothing for
    /// the user's own server, `unix://PATH`, or `npipe://./pipe/NAME`. A path alone is a
    /// Unix socket, and a name that begins `\\.\pipe\` is a pipe.
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
        // An address with a scheme this client does not speak is not a path.
        if address.contains("://") {
            return Err(bad());
        }
        if address.starts_with('/') {
            return Ok(Endpoint::Unix(PathBuf::from(address)));
        }
        Err(bad())
    }

    /// Whether the server is on this machine, reached without a network.
    pub fn is_on_this_machine(&self) -> bool {
        match self {
            Endpoint::Local | Endpoint::Pipe(_) | Endpoint::Unix(_) => true,
        }
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Endpoint::Local => f.write_str("local"),
            Endpoint::Pipe(name) => {
                let name = name.strip_prefix(r"\\.\pipe\").unwrap_or(name);
                write!(f, "npipe://./pipe/{name}")
            }
            Endpoint::Unix(path) => write!(f, "unix://{}", path.display()),
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
    fn an_address_is_written_as_it_is_read() {
        for address in [
            "local",
            "unix:///run/rex/server.sock",
            "npipe://./pipe/rex-S-1-5-21-1",
        ] {
            let endpoint = Endpoint::parse(address).unwrap();
            assert_eq!(endpoint.to_string(), address);
            assert_eq!(Endpoint::parse(&endpoint.to_string()).unwrap(), endpoint);
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
            "https://rex.example",
            "ssh://host",
            "rex",
        ] {
            let refused = Endpoint::parse(address).unwrap_err();
            assert_eq!(refused.code(), "bad_request", "{address}");
        }
    }
}
