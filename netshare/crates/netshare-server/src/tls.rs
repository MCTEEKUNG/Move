/// Server-side TLS: generates a self-signed certificate with `rcgen` and
/// builds a `tokio_rustls::TlsAcceptor` used by every TCP listener.
use std::sync::Arc;
use anyhow::Result;
use rcgen::{CertifiedKey, generate_simple_self_signed};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::TlsAcceptor;

use netshare_core::tls::cert_pairing_code;

/// Holds everything the server needs for TLS.
#[derive(Clone)]
pub struct ServerTls {
    pub acceptor: TlsAcceptor,
    /// DER-encoded certificate bytes (used to compute the pairing code).
    pub cert_der: Vec<u8>,
    /// 6-character hex pairing code derived from the cert fingerprint.
    /// Example: `"1A2B3C"`. Show this in the GUI; the client must enter it.
    pub pairing_code: String,
}

impl ServerTls {
    /// Generate a fresh self-signed TLS certificate and build the acceptor.
    pub fn generate() -> Result<Self> {
        let CertifiedKey { cert, key_pair } =
            generate_simple_self_signed(vec![netshare_core::tls::SERVER_NAME.to_owned()])?;

        let cert_der: Vec<u8> = cert.der().to_vec();
        let key_der: Vec<u8>  = key_pair.serialize_der();

        let pairing_code = cert_pairing_code(&cert_der);

        let cert_chain = vec![CertificateDer::from(cert_der.clone())];
        let private_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der));

        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, private_key)?;

        let acceptor = TlsAcceptor::from(Arc::new(config));

        Ok(Self { acceptor, cert_der, pairing_code })
    }
}
