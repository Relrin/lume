//! Encryption between this client and a server reached over a network.
//!
//! A server behind TLS is believed to be the one its address names when its
//! certificate says so and the system trusts who signed it. No certificate comes with
//! this application, and one that is not trusted is never passed over.

use std::io;
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::api::cut;
use crate::error::RexError;
use crate::tuning::{DIAL_TIMEOUT, TEXT_MAX_BYTES};

/// Whose word is taken that a server is the one its address names.
#[derive(Clone, Debug, Default)]
pub enum Trust {
    /// Whoever the system trusts: its own store of certificates, read as the system's
    /// own programs read it.
    #[default]
    System,
    /// Only who signed with one of these certificates, and nobody the system trusts. For
    /// a server whose certificates are signed by an authority that is nobody's but its
    /// owner's.
    Roots(Vec<CertificateDer<'static>>),
}

/// What a connection behind TLS is set up with.
pub(crate) fn config(trust: &Trust) -> Result<Arc<ClientConfig>, rustls::Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()?;
    let mut config = match trust {
        Trust::System => {
            let verifier = rustls_platform_verifier::Verifier::new(provider)?;
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(verifier))
                .with_no_client_auth()
        }
        Trust::Roots(certificates) => {
            let mut roots = RootCertStore::empty();
            for certificate in certificates {
                roots.add(certificate.clone())?;
            }
            builder.with_root_certificates(roots).with_no_client_auth()
        }
    };
    // A server speaks HTTP/1.1, which is what a WebSocket is asked for in.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// What went wrong when a connection behind TLS was begun: a certificate that is not
/// believed is the server's fault and stays so; anything else may pass.
fn failure(endpoint: &str, error: io::Error) -> RexError {
    let certificate = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .is_some_and(|inner| matches!(inner, rustls::Error::InvalidCertificate(_)));
    if certificate {
        let mut reason = error.to_string();
        cut(&mut reason, TEXT_MAX_BYTES);
        RexError::Untrusted {
            endpoint: endpoint.to_owned(),
            reason,
        }
    } else {
        RexError::Unreachable {
            endpoint: endpoint.to_owned(),
            source: error,
        }
    }
}

/// Begins TLS on a stream to the server that `host` names. `endpoint` names the server
/// in what is said of failures.
pub(crate) async fn begin(
    endpoint: &str,
    config: &Arc<ClientConfig>,
    host: &str,
    stream: TcpStream,
) -> Result<TlsStream<TcpStream>, RexError> {
    let name = ServerName::try_from(host.to_owned())
        .map_err(|_| RexError::BadEndpoint(endpoint.to_owned()))?;
    let connector = TlsConnector::from(Arc::clone(config));
    tokio::time::timeout(DIAL_TIMEOUT, connector.connect(name, stream))
        .await
        .map_err(|_| RexError::Unreachable {
            endpoint: endpoint.to_owned(),
            source: io::ErrorKind::TimedOut.into(),
        })?
        .map_err(|error| failure(endpoint, error))
}
