//! Fetching over HTTP and HTTPS, within what `net` grants.
//!
//! A grant names a server: `host`, `host:port`, `host:*`, `*.name` for every name under
//! one (not the name itself), an IPv4 address, `[an IPv6 address]`, or `*` for any. A
//! grant without a port means ports 80 and 443.
//!
//! The name is checked, and so is the address it leads to, the one actually connected
//! to: an address of this computer (loopback), of the local link, the unspecified
//! address, multicast or broadcast is refused unless the grant that allows the server
//! names that address itself, or names `localhost` for this computer. So a name that
//! leads to `127.0.0.1` (a DNS rebinding) is refused. Private networks are not refused:
//! a plugin granted a server on one may reach it.
//!
//! No proxy from the environment is used. Redirects are followed here, at most
//! [`MAX_REDIRECTS`], and each one is checked as the first request was; a redirect to
//! another server drops the request's `Authorization` and `Cookie` headers. Encryption
//! is rustls, trusting the certificates the system trusts.

use std::io::{self, Read};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, mpsc};
use std::time::{Duration, Instant};

use lume_plugin::permissions::split_host;
use serde::Serialize;
use ureq::http::{self, HeaderName, HeaderValue, Method, Uri};
use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{
    ConnectionDetails, Connector, DefaultConnector, NextTimeout, Transport,
};

use crate::scope::{Allowed, ServiceError};

/// The most redirects one fetch follows.
pub const MAX_REDIRECTS: usize = 5;

/// What a fetch says it is.
pub const USER_AGENT: &str = "Lume";

/// Headers a plugin cannot set: the ones that say how the connection is made or how the
/// body is framed, which are the client's to decide.
const REFUSED_HEADERS: [&str; 8] = [
    "connection",
    "content-length",
    "host",
    "keep-alive",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// A request a plugin makes.
pub struct FetchRequest {
    /// `GET`, `HEAD`, `POST`, `PUT`, `PATCH`, `DELETE` or `OPTIONS`.
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    /// For [`fetch`], how long the whole fetch may take, redirects and body included. For
    /// [`fetch_stream`], how long until the response's head, and then how long each read
    /// of the body may wait.
    pub timeout: Duration,
    /// The largest body taken.
    pub max_bytes: u64,
}

impl std::fmt::Debug for FetchRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Headers and the body can hold secrets, so only their sizes are shown.
        f.debug_struct("FetchRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &self.headers.len())
            .field("body", &self.body.as_ref().map(Vec::len))
            .field("timeout", &self.timeout)
            .field("max_bytes", &self.max_bytes)
            .finish()
    }
}

/// A response's status and headers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FetchHead {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// Where the response came from, after any redirects.
    pub url: String,
}

/// A whole response.
pub struct FetchResponse {
    pub head: FetchHead,
    pub body: Vec<u8>,
}

impl std::fmt::Debug for FetchResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FetchResponse")
            .field("head", &self.head)
            .field("body", &self.body.len())
            .finish()
    }
}

/// A response's head and a reader of its body.
pub type Streamed = (FetchHead, Box<dyn Read + Send>);

/// A way to look a host name up: the addresses of a name, each with the port given.
pub type Lookup = dyn Fn(&str, u16) -> io::Result<Vec<SocketAddr>> + Send + Sync;

/// The system's own lookup.
fn system_lookup() -> Arc<Lookup> {
    Arc::new(|host: &str, port: u16| (host, port).to_socket_addrs().map(Iterator::collect))
}

/// Fetches a URL and its whole body, once the server is checked against `granted` (the
/// servers of `net`). The grant given back is the one that allowed the server the
/// response came from, after any redirects.
pub fn fetch(
    granted: &[String],
    request: FetchRequest,
) -> Result<Allowed<FetchResponse>, ServiceError> {
    fetch_with(&system_lookup(), granted, request)
}

/// [`fetch`], looking host names up with `lookup` in place of the system.
pub fn fetch_with(
    lookup: &Arc<Lookup>,
    granted: &[String],
    request: FetchRequest,
) -> Result<Allowed<FetchResponse>, ServiceError> {
    let max_bytes = request.max_bytes;
    let exchange = exchange(lookup, granted, request, Mode::Whole)?;
    let body = exchange
        .body
        .into_with_config()
        .limit(max_bytes)
        .read_to_vec()
        .map_err(|error| fetch_error(error, &exchange.head.url))?;
    Ok(Allowed {
        value: FetchResponse {
            head: exchange.head,
            body,
        },
        grant: exchange.grant,
    })
}

/// Fetches a URL and gives its head and a reader of its body, once the server is
/// checked against `granted` (the servers of `net`). The reader fails once the body
/// goes past `max_bytes`.
pub fn fetch_stream(
    granted: &[String],
    request: FetchRequest,
) -> Result<Allowed<Streamed>, ServiceError> {
    fetch_stream_with(&system_lookup(), granted, request)
}

/// [`fetch_stream`], looking host names up with `lookup` in place of the system.
pub fn fetch_stream_with(
    lookup: &Arc<Lookup>,
    granted: &[String],
    request: FetchRequest,
) -> Result<Allowed<Streamed>, ServiceError> {
    let max_bytes = request.max_bytes;
    let exchange = exchange(lookup, granted, request, Mode::Stream)?;
    let reader = Capped {
        inner: exchange.body.into_with_config().limit(max_bytes).reader(),
        max_bytes,
    };
    Ok(Allowed {
        value: (exchange.head, Box::new(reader)),
        grant: exchange.grant,
    })
}

/// A body's reader that says plainly when the body is too large.
struct Capped {
    inner: ureq::BodyReader<'static>,
    max_bytes: u64,
}

impl Read for Capped {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer).map_err(|error| {
            let too_large = error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<ureq::Error>())
                .is_some_and(|inner| matches!(inner, ureq::Error::BodyExceedsLimit(_)));
            if too_large {
                io::Error::new(
                    io::ErrorKind::FileTooLarge,
                    format!("the response is larger than {} bytes", self.max_bytes),
                )
            } else {
                error
            }
        })
    }
}

/// The grant among `granted` (the servers of `net`) that allows `host` on `port`, if
/// any. The host is a name or an address, an IPv6 one with or without brackets.
pub fn host_allowed(granted: &[String], host: &str, port: u16) -> Option<String> {
    matching(granted, host, port)
        .into_iter()
        .next()
        .map(str::to_owned)
}

/// Every grant that allows `host` on `port`.
fn matching<'a>(granted: &'a [String], host: &str, port: u16) -> Vec<&'a str> {
    let host = host.to_ascii_lowercase();
    granted
        .iter()
        .map(String::as_str)
        .filter(|grant| grant_allows(grant, &host, port))
        .collect()
}

/// Whether one grant allows a host, written in lower case, on a port.
fn grant_allows(grant: &str, host: &str, port: u16) -> bool {
    let grant = grant.to_ascii_lowercase();
    let Some((name, ports)) = split_host(&grant) else {
        return false;
    };
    let port_allowed = match ports {
        None => port == 80 || port == 443,
        Some("*") => true,
        Some(written) => written.parse::<u16>() == Ok(port),
    };
    if !port_allowed {
        return false;
    }
    if name == "*" {
        return true;
    }
    match literal_ip(host) {
        // An address is allowed only by a grant naming that address.
        Some(address) => literal_ip(name).is_some_and(|named| same_address(named, address)),
        None => match name.strip_prefix("*.") {
            Some(parent) => host
                .strip_suffix(parent)
                .is_some_and(|under| under.len() > 1 && under.ends_with('.')),
            None => literal_ip(name).is_none() && host == name,
        },
    }
}

/// The address a host is written as, if it is one; an IPv6 one with or without
/// brackets.
fn literal_ip(host: &str) -> Option<IpAddr> {
    let bare = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host);
    bare.parse().ok()
}

/// An IPv6 address that carries an IPv4 one, as that IPv4 address; any other as it is.
fn plain(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return IpAddr::V4(v4);
            }
            // The old form `::a.b.c.d`; `::` and `::1` are not of it.
            let segments = v6.segments();
            if segments[..6].iter().all(|segment| *segment == 0)
                && !(segments[6] == 0 && segments[7] <= 1)
            {
                let [.., high, low] = segments;
                let [a, b] = high.to_be_bytes();
                let [c, d] = low.to_be_bytes();
                return IpAddr::V4(Ipv4Addr::new(a, b, c, d));
            }
            address
        }
        IpAddr::V4(_) => address,
    }
}

fn same_address(a: IpAddr, b: IpAddr) -> bool {
    a == b || plain(a) == plain(b)
}

/// Whether an address is one no grant reaches without naming it: this computer's, the
/// local link's, the unspecified address, multicast or broadcast.
pub fn is_special_address(address: IpAddr) -> bool {
    match plain(address) {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_link_local()
                || v4.octets()[0] == 0
                || v4.is_multicast()
                || v4.is_broadcast()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.segments()[0] & 0xffc0 == 0xfe80
        }
    }
}

/// Whether an address is this computer's.
fn is_loopback(address: IpAddr) -> bool {
    match plain(address) {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

/// Whether one of the grants that allow a server allows connecting to `address`.
fn address_allowed(grants: &[&str], address: IpAddr) -> bool {
    if !is_special_address(address) {
        return true;
    }
    grants.iter().any(|grant| {
        let name = split_host(grant).map_or(*grant, |(name, _)| name);
        match literal_ip(name) {
            Some(named) => same_address(named, address),
            None => name.eq_ignore_ascii_case("localhost") && is_loopback(address),
        }
    })
}

/// Where a request goes.
#[derive(Clone, Debug)]
struct Target {
    uri: Uri,
    text: String,
    https: bool,
    host: String,
    port: u16,
}

impl Target {
    fn parse(written: &str) -> Result<Self, ServiceError> {
        let text = written.split('#').next().unwrap_or("");
        let only_http =
            || ServiceError::invalid(format!("`{written}`: only http and https can be fetched"));
        let scheme = text
            .split_once("://")
            .map(|(scheme, _)| scheme.to_ascii_lowercase());
        let https = match scheme.as_deref() {
            Some("https") => true,
            Some("http") => false,
            _ if text.contains(char::is_whitespace) => {
                return Err(ServiceError::invalid(format!("`{written}` is not a URL")));
            }
            _ => return Err(only_http()),
        };
        let uri: Uri = text
            .parse()
            .map_err(|_| ServiceError::invalid(format!("`{written}` is not a URL")))?;
        if !uri
            .scheme_str()
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case(if https { "https" } else { "http" }))
        {
            return Err(only_http());
        }
        let authority = uri
            .authority()
            .ok_or_else(|| ServiceError::invalid(format!("`{written}` names no server")))?;
        if authority.as_str().contains('@') {
            return Err(ServiceError::invalid(format!(
                "`{written}`: a URL cannot hold a user name or password"
            )));
        }
        let host = authority.host().to_ascii_lowercase();
        if host.is_empty() {
            return Err(ServiceError::invalid(format!(
                "`{written}` names no server"
            )));
        }
        let port = authority.port_u16().unwrap_or(if https { 443 } else { 80 });
        Ok(Self {
            text: uri.to_string(),
            uri,
            https,
            host,
            port,
        })
    }

    /// Whether another target is the same server, as a browser decides what to send to
    /// it.
    fn same_origin(&self, other: &Self) -> bool {
        self.https == other.https && self.host == other.host && self.port == other.port
    }

    /// Where a redirect's `Location` leads from here.
    fn follow(&self, location: &str) -> String {
        let location = location.trim().split('#').next().unwrap_or("");
        let scheme = if self.https { "https" } else { "http" };
        let authority = self
            .uri
            .authority()
            .map_or("", |authority| authority.as_str());
        let has_scheme = location.find(':').is_some_and(|colon| {
            location[..colon]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                && !location[..colon].is_empty()
                && !location[..colon].contains('/')
        });
        if has_scheme {
            return location.to_owned();
        }
        if location.starts_with("//") {
            return format!("{scheme}:{location}");
        }
        if location.is_empty() {
            return self.text.clone();
        }
        let (path, query) = match location.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (location, None),
        };
        let base = self.uri.path();
        let merged = if path.is_empty() {
            base.to_owned()
        } else if path.starts_with('/') {
            path.to_owned()
        } else {
            let folder = base.rfind('/').map_or("/", |end| &base[..=end]);
            format!("{folder}{path}")
        };
        let query = match (query, path.is_empty()) {
            (Some(query), _) => format!("?{query}"),
            (None, true) => self
                .uri
                .query()
                .map(|query| format!("?{query}"))
                .unwrap_or_default(),
            (None, false) => String::new(),
        };
        format!("{scheme}://{authority}{}{query}", remove_dots(&merged))
    }
}

/// A path without its `.` and `..` names, as a URL's path is resolved.
fn remove_dots(path: &str) -> String {
    let names: Vec<&str> = path.split('/').skip(1).collect();
    let mut kept: Vec<&str> = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let last = index + 1 == names.len();
        match *name {
            "." => {}
            ".." => {
                kept.pop();
            }
            name => {
                kept.push(name);
                continue;
            }
        }
        if last {
            kept.push("");
        }
    }
    format!("/{}", kept.join("/"))
}

fn parse_method(method: &str) -> Result<Method, ServiceError> {
    let upper = method.to_ascii_uppercase();
    match upper.as_str() {
        "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS" => {
            Method::from_bytes(upper.as_bytes())
                .map_err(|_| ServiceError::invalid(format!("`{method}` is not a method")))
        }
        _ => Err(ServiceError::invalid(format!(
            "`{method}` is not a method a plugin can use: GET, HEAD, POST, PUT, PATCH, DELETE \
             or OPTIONS"
        ))),
    }
}

/// The plugin's headers, checked. A value is never put in a message: it may be a
/// token.
fn check_headers(
    headers: &[(String, String)],
) -> Result<Vec<(HeaderName, HeaderValue)>, ServiceError> {
    headers
        .iter()
        .map(|(name, value)| {
            let header = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| ServiceError::invalid(format!("`{name}` is not a header's name")))?;
            let lower = header.as_str();
            if REFUSED_HEADERS.contains(&lower) || lower.starts_with("proxy-") {
                return Err(ServiceError::invalid(format!(
                    "a plugin cannot set the `{name}` header"
                )));
            }
            let value = HeaderValue::from_str(value).map_err(|_| {
                ServiceError::invalid(format!(
                    "the value given for `{name}` is not one a header can have"
                ))
            })?;
            Ok((header, value))
        })
        .collect()
}

/// How a response's body is to be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// All at once, within the timeout.
    Whole,
    /// Bit by bit, each read within the timeout.
    Stream,
}

/// A response with its head read, from the server a grant allowed.
struct Exchange {
    head: FetchHead,
    body: ureq::Body,
    grant: String,
}

/// Sends a request and follows its redirects, checking every one.
fn exchange(
    lookup: &Arc<Lookup>,
    granted: &[String],
    request: FetchRequest,
    mode: Mode,
) -> Result<Exchange, ServiceError> {
    let mut method = parse_method(&request.method)?;
    let mut headers = check_headers(&request.headers)?;
    let mut body = request.body;
    let mut target = Target::parse(&request.url)?;
    let deadline = Instant::now() + request.timeout;
    let mut redirects = 0;
    loop {
        let grant = host_allowed(granted, &target.host, target.port).ok_or_else(|| {
            ServiceError::denied(format!(
                "no net grant covers {}:{} for `{}`",
                target.host, target.port, target.text
            ))
        })?;
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ServiceError::timed_out(format!(
                "`{}` did not answer in time",
                target.text
            )));
        }
        let timeouts = Timeouts {
            left,
            each_read: request.timeout,
            mode,
        };
        let response = send(
            lookup,
            granted,
            &target,
            &method,
            &headers,
            body.as_deref(),
            timeouts,
        )?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get(http::header::LOCATION)
            .and_then(|location| location.to_str().ok())
            .map(str::to_owned);
        if let (301 | 302 | 303 | 307 | 308, Some(location)) = (status, location) {
            if redirects == MAX_REDIRECTS {
                return Err(ServiceError::failed(format!(
                    "`{}` redirected more than {MAX_REDIRECTS} times",
                    request.url
                )));
            }
            redirects += 1;
            let next = Target::parse(&target.follow(&location)).map_err(|_| {
                ServiceError::denied(format!(
                    "`{}` redirected to `{location}`, which cannot be fetched",
                    target.text
                ))
            })?;
            // As browsers do: after 303, and after 301 or 302 to a POST, the next
            // request is a GET without the body.
            let to_get = (status == 303 && method != Method::HEAD)
                || (matches!(status, 301 | 302) && method == Method::POST);
            if to_get {
                method = Method::GET;
                body = None;
                headers.retain(|(name, _)| {
                    !matches!(
                        name.as_str(),
                        "content-type"
                            | "content-encoding"
                            | "content-language"
                            | "content-location"
                    )
                });
            }
            if !target.same_origin(&next) {
                headers.retain(|(name, _)| {
                    *name != http::header::AUTHORIZATION && *name != http::header::COOKIE
                });
            }
            target = next;
            continue;
        }
        let (parts, body) = response.into_parts();
        let head = FetchHead {
            status,
            headers: parts
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        String::from_utf8_lossy(value.as_bytes()).into_owned(),
                    )
                })
                .collect(),
            url: target.text,
        };
        return Ok(Exchange { head, body, grant });
    }
}

/// How long one request may take.
#[derive(Clone, Copy, Debug)]
struct Timeouts {
    /// What is left of the whole fetch's time.
    left: Duration,
    /// How long each read of a streamed body may wait.
    each_read: Duration,
    mode: Mode,
}

/// The agent's settings for one request.
fn config(timeouts: Timeouts) -> ureq::config::Config {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let builder = ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .http_status_as_error(false)
        .save_redirect_history(false)
        .user_agent(USER_AGENT)
        .tls_config(tls);
    let left = Some(timeouts.left);
    match timeouts.mode {
        Mode::Whole => builder.timeout_global(left),
        Mode::Stream => builder
            .timeout_resolve(left)
            .timeout_connect(left)
            .timeout_send_request(left)
            .timeout_send_body(left)
            .timeout_recv_response(left)
            .timeout_per_read(Some(timeouts.each_read)),
    }
    .build()
}

/// The connector every fetch shares, so encryption is set up once.
fn connector() -> Arc<DefaultConnector> {
    static SHARED: OnceLock<Arc<DefaultConnector>> = OnceLock::new();
    Arc::clone(SHARED.get_or_init(|| Arc::new(DefaultConnector::new())))
}

/// The shared connector, lent to one agent.
#[derive(Debug)]
struct Shared(Arc<DefaultConnector>);

impl Connector<()> for Shared {
    type Out = Box<dyn Transport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        self.0.connect(details, chained)
    }
}

/// Looks a server up and keeps only the addresses its grants allow connecting to. The
/// agent connects only to what this gives.
struct Checked {
    lookup: Arc<Lookup>,
    granted: Vec<String>,
    /// Why the lookup was refused, for the fetch to say.
    refused: Arc<Mutex<Option<ServiceError>>>,
}

impl std::fmt::Debug for Checked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Checked")
            .field("granted", &self.granted)
            .finish_non_exhaustive()
    }
}

impl Checked {
    fn refuse(&self, error: ServiceError) -> ureq::Error {
        *self.refused.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
        ureq::Error::Io(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the address is not allowed",
        ))
    }

    fn look_up(
        &self,
        host: &str,
        port: u16,
        timeout: NextTimeout,
    ) -> Result<Vec<SocketAddr>, ureq::Error> {
        if timeout.after.is_not_happening() {
            return (self.lookup)(host, port).map_err(|_| ureq::Error::HostNotFound);
        }
        let (tell, answer) = mpsc::sync_channel(1);
        let lookup = Arc::clone(&self.lookup);
        let name = host.to_owned();
        std::thread::spawn(move || {
            let _ = tell.send(lookup(&name, port));
        });
        match answer.recv_timeout(*timeout.after) {
            Ok(Ok(addresses)) => Ok(addresses),
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(ureq::Error::HostNotFound)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Err(ureq::Error::Timeout(timeout.reason)),
        }
    }
}

impl Resolver for Checked {
    fn resolve(
        &self,
        uri: &Uri,
        _config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let host = uri.host().unwrap_or("").to_ascii_lowercase();
        let https = uri.scheme_str() == Some("https");
        let port = uri.port_u16().unwrap_or(if https { 443 } else { 80 });
        let grants = matching(&self.granted, &host, port);
        if grants.is_empty() {
            return Err(self.refuse(ServiceError::denied(format!(
                "no net grant covers {host}:{port}"
            ))));
        }
        let addresses = match literal_ip(&host) {
            Some(address) => vec![SocketAddr::new(address, port)],
            None => self.look_up(&host, port, timeout)?,
        };
        let mut allowed = self.empty();
        let mut refused = None;
        for address in addresses {
            if !address_allowed(&grants, address.ip()) {
                refused.get_or_insert(address.ip());
                continue;
            }
            if allowed.try_push(address).is_err() {
                break;
            }
        }
        if allowed.is_empty() {
            return Err(match refused {
                Some(address) => self.refuse(ServiceError::denied(format!(
                    "{host} leads to {address}, an address of this computer or its local link \
                     that no grant names"
                ))),
                None => ureq::Error::HostNotFound,
            });
        }
        Ok(allowed)
    }
}

/// Sends one request to a checked target.
fn send(
    lookup: &Arc<Lookup>,
    granted: &[String],
    target: &Target,
    method: &Method,
    headers: &[(HeaderName, HeaderValue)],
    body: Option<&[u8]>,
    timeouts: Timeouts,
) -> Result<http::Response<ureq::Body>, ServiceError> {
    let refused = Arc::new(Mutex::new(None));
    let resolver = Checked {
        lookup: Arc::clone(lookup),
        granted: granted.to_vec(),
        refused: Arc::clone(&refused),
    };
    let agent = ureq::Agent::with_parts(config(timeouts), Shared(connector()), resolver);
    let mut builder = http::Request::builder()
        .method(method.clone())
        .uri(target.uri.clone());
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    let unusable = |_| ServiceError::invalid(format!("`{}` cannot be requested", target.text));
    let result = match body {
        Some(body) => agent.run(builder.body(body).map_err(unusable)?),
        None => agent.run(builder.body(()).map_err(unusable)?),
    };
    result.map_err(|error| {
        if let Some(refusal) = refused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            return refusal;
        }
        fetch_error(error, &target.text)
    })
}

/// What went wrong with a fetch, for the plugin.
fn fetch_error(error: ureq::Error, url: &str) -> ServiceError {
    match error {
        ureq::Error::Timeout(_) => {
            ServiceError::timed_out(format!("`{url}` did not answer in time"))
        }
        ureq::Error::Io(error) if error.kind() == io::ErrorKind::TimedOut => {
            ServiceError::timed_out(format!("`{url}` did not answer in time"))
        }
        ureq::Error::HostNotFound => {
            ServiceError::not_found(format!("the server of `{url}` could not be found"))
        }
        ureq::Error::BodyExceedsLimit(limit) => ServiceError::too_large(format!(
            "the response from `{url}` is larger than {limit} bytes"
        )),
        ureq::Error::ConnectionFailed => {
            ServiceError::failed(format!("the server of `{url}` could not be reached"))
        }
        other => ServiceError::failed(format!("fetching `{url}` failed: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv6Addr;

    use super::*;

    fn grants(list: &[&str]) -> Vec<String> {
        list.iter().map(|grant| (*grant).to_owned()).collect()
    }

    #[test]
    fn a_host_without_a_port_means_ports_80_and_443() {
        let granted = grants(&["api.github.com"]);
        assert_eq!(
            host_allowed(&granted, "api.github.com", 443).as_deref(),
            Some("api.github.com")
        );
        assert!(host_allowed(&granted, "api.github.com", 80).is_some());
        assert!(host_allowed(&granted, "api.github.com", 8443).is_none());
        assert!(host_allowed(&granted, "API.GitHub.com", 443).is_some());
        assert!(host_allowed(&granted, "github.com", 443).is_none());
        assert!(host_allowed(&granted, "evil-api.github.com", 443).is_none());
    }

    #[test]
    fn a_port_is_one_port_or_any() {
        let granted = grants(&["localhost:8080", "db.local:*"]);
        assert!(host_allowed(&granted, "localhost", 8080).is_some());
        assert!(host_allowed(&granted, "localhost", 8081).is_none());
        assert!(host_allowed(&granted, "localhost", 80).is_none());
        assert!(host_allowed(&granted, "db.local", 1).is_some());
        assert!(host_allowed(&granted, "db.local", 65535).is_some());
    }

    #[test]
    fn a_star_name_covers_every_name_under_it_but_not_itself() {
        let granted = grants(&["*.example.org"]);
        assert!(host_allowed(&granted, "a.example.org", 443).is_some());
        assert!(host_allowed(&granted, "a.b.example.org", 443).is_some());
        assert!(host_allowed(&granted, "example.org", 443).is_none());
        assert!(host_allowed(&granted, "badexample.org", 443).is_none());
        assert!(host_allowed(&granted, ".example.org", 443).is_none());
    }

    #[test]
    fn an_address_is_covered_only_by_a_grant_naming_it() {
        let granted = grants(&["10.0.0.5", "[::1]:9000", "*.0.0.5"]);
        assert!(host_allowed(&granted, "10.0.0.5", 80).is_some());
        assert!(host_allowed(&granted, "10.0.0.6", 80).is_none());
        // A name made of digits is not an address, and an address is not a name.
        assert!(host_allowed(&grants(&["*.0.0.5"]), "10.0.0.5", 80).is_none());
        assert!(host_allowed(&granted, "[::1]", 9000).is_some());
        assert!(host_allowed(&granted, "::1", 9000).is_some());
        assert!(host_allowed(&granted, "[0:0::1]", 9000).is_some());
        assert!(host_allowed(&granted, "[::2]", 9000).is_none());
        assert!(host_allowed(&grants(&["127.0.0.1"]), "[::ffff:127.0.0.1]", 80).is_some());
    }

    #[test]
    fn a_star_alone_covers_any_server() {
        let granted = grants(&["*"]);
        assert!(host_allowed(&granted, "anything.example", 443).is_some());
        assert!(host_allowed(&granted, "203.0.113.9", 80).is_some());
        assert!(host_allowed(&granted, "anything.example", 8080).is_none());
        assert!(host_allowed(&grants(&["*:*"]), "anything.example", 8080).is_some());
    }

    #[test]
    fn local_addresses_are_told_from_the_rest() {
        let special = [
            "127.0.0.1",
            "127.255.0.9",
            "::1",
            "169.254.169.254",
            "fe80::1",
            "0.0.0.0",
            "0.1.2.3",
            "::",
            "224.0.0.1",
            "ff02::1",
            "255.255.255.255",
            "::ffff:127.0.0.1",
            "::ffff:169.254.1.1",
            "::127.0.0.1",
        ];
        for address in special {
            let address: IpAddr = address.parse().unwrap();
            assert!(is_special_address(address), "{address}");
        }
        let ordinary = [
            "10.0.0.1",
            "192.168.1.10",
            "172.16.5.4",
            "203.0.113.9",
            "fd00::1",
            "2001:db8::1",
            "::ffff:10.0.0.1",
        ];
        for address in ordinary {
            let address: IpAddr = address.parse().unwrap();
            assert!(!is_special_address(address), "{address}");
        }
    }

    #[test]
    fn a_local_address_needs_a_grant_naming_it_or_localhost() {
        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let v6 = IpAddr::V6(Ipv6Addr::LOCALHOST);
        assert!(address_allowed(&["127.0.0.1:8080"], loopback));
        assert!(address_allowed(&["localhost:8080"], loopback));
        assert!(address_allowed(&["localhost"], v6));
        assert!(address_allowed(&["[::1]:8080"], v6));
        assert!(!address_allowed(&["*"], loopback));
        assert!(!address_allowed(&["*.example.org"], loopback));
        assert!(!address_allowed(&["127.0.0.2"], loopback));
        assert!(!address_allowed(
            &["localhost"],
            "169.254.169.254".parse().unwrap()
        ));
        assert!(address_allowed(&["*"], "10.0.0.1".parse().unwrap()));
        assert!(address_allowed(
            &["127.0.0.1"],
            "::ffff:127.0.0.1".parse().unwrap()
        ));
    }

    #[test]
    fn a_redirect_is_read_against_where_it_came_from() {
        let target = Target::parse("https://a.example/one/two?x=1").unwrap();
        assert_eq!(target.follow("https://b.example/z"), "https://b.example/z");
        assert_eq!(target.follow("//c.example/y"), "https://c.example/y");
        assert_eq!(target.follow("/root?q=2"), "https://a.example/root?q=2");
        assert_eq!(target.follow("three"), "https://a.example/one/three");
        assert_eq!(target.follow("../up"), "https://a.example/up");
        assert_eq!(target.follow("./same/"), "https://a.example/one/same/");
        assert_eq!(target.follow("?y=3"), "https://a.example/one/two?y=3");
        assert_eq!(target.follow("../../../../top"), "https://a.example/top");
        assert_eq!(target.follow("file:///etc/passwd"), "file:///etc/passwd");
        assert_eq!(target.follow("/a#frag"), "https://a.example/a");
    }

    #[test]
    fn only_http_and_https_urls_without_credentials_are_fetched() {
        for (url, says) in [
            ("ftp://example.org/x", "only http and https"),
            ("file:///etc/passwd", "only http and https"),
            ("http://user:pw@example.org/", "user name or password"),
            ("not a url", "not a URL"),
            ("/relative", "only http and https"),
        ] {
            let error = Target::parse(url).unwrap_err();
            assert_eq!(error.kind, crate::ErrorKind::Invalid, "{url}");
            assert!(error.message.contains(says), "{url}: {}", error.message);
        }
        let target = Target::parse("HTTP://Example.ORG:8080/a#b").unwrap();
        assert_eq!(
            (target.host.as_str(), target.port, target.https),
            ("example.org", 8080, false)
        );
    }

    #[test]
    fn a_plugin_cannot_set_the_headers_that_frame_the_connection() {
        for name in [
            "Host",
            "connection",
            "Content-Length",
            "Transfer-Encoding",
            "Proxy-Authorization",
            "proxy-anything",
            "Upgrade",
        ] {
            let error = check_headers(&[(name.to_owned(), "x".to_owned())]).unwrap_err();
            assert_eq!(error.kind, crate::ErrorKind::Invalid, "{name}");
        }
        let bad =
            check_headers(&[("X-Token".to_owned(), "abc\r\nInjected: 1".to_owned())]).unwrap_err();
        assert!(!bad.message.contains("abc"));
        assert!(check_headers(&[("Authorization".to_owned(), "Bearer x".to_owned())]).is_ok());
        assert!(check_headers(&[("bad name".to_owned(), "x".to_owned())]).is_err());
    }

    #[test]
    fn only_the_common_methods_are_allowed() {
        assert_eq!(parse_method("get").unwrap(), Method::GET);
        assert_eq!(parse_method("PATCH").unwrap(), Method::PATCH);
        for method in ["CONNECT", "TRACE", "PROPFIND", ""] {
            assert!(parse_method(method).is_err(), "{method}");
        }
    }
}
