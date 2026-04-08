/// Shared TLS utilities used by both server and client crates.
///
/// The server generates a self-signed certificate with `rcgen`.
/// The client uses `AcceptAnyCert` — security comes from the pairing code
/// that the user manually confirms out-of-band.
use std::sync::Arc;

use rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

/// The SNI name embedded in the server's self-signed certificate.
/// All client connections use this as the TLS server-name.
pub const SERVER_NAME: &str = "netshare.local";

// ── Client: trust-any verifier ─────────────────────────────────────────────

/// A TLS certificate verifier that accepts *any* server certificate.
///
/// Security is provided by the pairing code the user confirms manually.
/// After pairing, the cert fingerprint could be pinned (future enhancement).
#[derive(Debug)]
pub struct AcceptAnyCert;

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Build a `rustls::ClientConfig` that accepts any server certificate.
/// Use this to create a `tokio_rustls::TlsConnector` in the server/client crates.
pub fn make_client_config() -> rustls::ClientConfig {
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
        .with_no_client_auth()
}

/// Compute a short human-readable fingerprint from a DER-encoded certificate.
///
/// Returns the first 3 bytes of SHA-256 encoded as 6 uppercase hex characters,
/// e.g. `"1A2B3C"`. This is shown as the pairing code.
pub fn cert_pairing_code(cert_der: &[u8]) -> String {
    let digest = Sha256::digest(cert_der);
    format!("{:02X}{:02X}{:02X}", digest[0], digest[1], digest[2])
}
