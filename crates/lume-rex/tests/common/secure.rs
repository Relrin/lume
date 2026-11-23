//! A pretend server behind TLS: an authority made for one test, the certificates it
//! signs, and a door that speaks TLS before it hands a stream to the pretend server.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use rcgen::{
    BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, KeyUsagePurpose, date_time_ymd,
};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

use super::{Listener, STREAM_BYTES};

/// When a certificate holds.
#[derive(Clone, Copy)]
pub enum Valid {
    Now,
    /// It stopped holding years ago.
    Once,
}

/// Somebody who signs certificates, whom nobody trusts but a test that says so.
pub struct Authority {
    issuer: Issuer<'static, KeyPair>,
    /// Its own certificate: what a client that trusts it is given.
    pub root: CertificateDer<'static>,
}

fn params(names: &[&str], valid: Valid) -> CertificateParams {
    let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    let mut params = CertificateParams::new(names).unwrap();
    if let Valid::Once = valid {
        params.not_before = date_time_ymd(2019, 1, 1);
        params.not_after = date_time_ymd(2020, 1, 1);
    }
    params
}

fn config(certificate: CertificateDer<'static>, key: &KeyPair) -> Arc<ServerConfig> {
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)
        .unwrap();
    Arc::new(config)
}

impl Authority {
    pub fn new() -> Self {
        let key = KeyPair::generate().unwrap();
        let mut params = params(&[], Valid::Now);
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        let root = params.self_signed(&key).unwrap().der().clone();
        Self {
            issuer: Issuer::new(params, key),
            root,
        }
    }

    /// What a server is set up with whose certificate this authority signed for `names`.
    pub fn server(&self, names: &[&str], valid: Valid) -> Arc<ServerConfig> {
        let key = KeyPair::generate().unwrap();
        let certificate = params(names, valid).signed_by(&key, &self.issuer).unwrap();
        config(certificate.der().clone(), &key)
    }
}

/// What a server is set up with whose certificate nobody signed but itself.
pub fn self_signed(names: &[&str]) -> Arc<ServerConfig> {
    let key = KeyPair::generate().unwrap();
    let certificate = params(names, Valid::Now).self_signed(&key).unwrap();
    config(certificate.der().clone(), &key)
}

/// The names the clients of a door asked for the server by as TLS began, in order.
pub type Named = Arc<Mutex<Vec<Option<String>>>>;

/// A pretend server's door on this machine's own network, behind TLS. A connection is
/// one more stream of the listener once TLS has begun on it; one whose client does not
/// believe the certificate never is.
pub async fn door(config: Arc<ServerConfig>) -> (SocketAddr, Listener, Named) {
    let door = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = door.local_addr().unwrap();
    let (servers, taken) = mpsc::unbounded_channel();
    let named = Named::default();
    let noted = Arc::clone(&named);
    let acceptor = TlsAcceptor::from(config);
    tokio::spawn(async move {
        while let Ok((stream, _)) = door.accept().await {
            let acceptor = acceptor.clone();
            let servers = servers.clone();
            let noted = Arc::clone(&noted);
            tokio::spawn(async move {
                let Ok(mut stream) = acceptor.accept(stream).await else {
                    return;
                };
                let name = stream.get_ref().1.server_name().map(str::to_owned);
                noted.lock().unwrap().push(name);
                let (mut near, far) = tokio::io::duplex(STREAM_BYTES);
                if servers.send(far).is_ok() {
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut near).await;
                }
            });
        }
    });
    (address, Listener { servers: taken }, named)
}

/// A door behind TLS with something that is not a Rex server behind it: whatever is
/// asked, it has not got. Gives its address and how many were let in.
pub async fn not_rex(config: Arc<ServerConfig>) -> (SocketAddr, Arc<Mutex<usize>>) {
    let door = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = door.local_addr().unwrap();
    let asked = Arc::new(Mutex::new(0));
    let counted = Arc::clone(&asked);
    let acceptor = TlsAcceptor::from(config);
    tokio::spawn(async move {
        while let Ok((stream, _)) = door.accept().await {
            let acceptor = acceptor.clone();
            let counted = Arc::clone(&counted);
            tokio::spawn(async move {
                let Ok(mut stream) = acceptor.accept(stream).await else {
                    return;
                };
                *counted.lock().unwrap() += 1;
                let mut request = [0; 2048];
                let _ = stream.read(&mut request).await;
                let _ = stream
                    .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
                    .await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (address, asked)
}

/// The server's side of a connection that only asks whether a Rex server is there: the
/// handshake of a control connection, and then nothing but its close.
// The library that shakes hands says what a refusal is, and it is not small.
#[allow(clippy::result_large_err)]
pub async fn probed(stream: DuplexStream) {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;

    let mut socket = accept_hdr_async(
        stream,
        |request: &Request, mut response: Response| -> Result<Response, ErrorResponse> {
            assert_eq!(request.uri().path(), "/api/control");
            if let Some(protocol) = request.headers().get("Sec-WebSocket-Protocol").cloned() {
                response
                    .headers_mut()
                    .insert("Sec-WebSocket-Protocol", protocol);
            }
            Ok(response)
        },
    )
    .await
    .unwrap();
    loop {
        match socket.next().await {
            None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
            Some(Ok(Message::Text(text))) => panic!("something was said after all: {text}"),
            Some(Ok(_)) => {}
        }
    }
}
