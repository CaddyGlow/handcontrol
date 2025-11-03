use crate::{certificates::CertificatePaths, storage::ServerRegistryEntry};
use anyhow::{anyhow, bail, Context, Result};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime},
    ClientConfig, SignatureScheme,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::sync::Arc;

/// Placeholder type for the future gRPC client.
pub struct HandControlClient;

impl HandControlClient {
    /// Validate TLS materials and return a stub error until the transport wiring lands.
    pub async fn connect(
        address: &str,
        port: u16,
        cert_paths: &CertificatePaths,
        server_fingerprint: &str,
    ) -> Result<Self> {
        let _ = build_tls_config(cert_paths, server_fingerprint)
            .with_context(|| format!("Failed to prepare TLS config for {address}:{port}"))?;

        Err(anyhow!(
            "handcontrol CLI/TUI gRPC transport is not implemented yet (TLS wiring pending)"
        ))
    }
}

fn build_tls_config(
    cert_paths: &CertificatePaths,
    server_fingerprint: &str,
) -> Result<Arc<ClientConfig>> {
    let client_cert_pem = fs::read(&cert_paths.client_cert)
        .with_context(|| format!("Failed to read {}", cert_paths.client_cert.display()))?;
    let client_key_pem = fs::read(&cert_paths.client_key)
        .with_context(|| format!("Failed to read {}", cert_paths.client_key.display()))?;

    let cert_chain =
        load_certs(&client_cert_pem).context("Failed to parse client certificate chain")?;
    let client_key =
        load_private_key(&client_key_pem).context("Failed to parse client private key")?;

    let verifier = Arc::new(PinnedServerCertVerifier::new(server_fingerprint)?);

    let mut config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(cert_chain, client_key)?;

    config.alpn_protocols.push(b"h2".to_vec());
    Ok(Arc::new(config))
}

fn load_certs(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>> {
    let mut reader = std::io::Cursor::new(pem);
    let mut certs = Vec::new();
    for result in rustls_pemfile::certs(&mut reader) {
        certs.push(result?);
    }
    if certs.is_empty() {
        bail!("No certificates found in PEM");
    }
    Ok(certs)
}

fn load_private_key(pem: &[u8]) -> Result<PrivateKeyDer<'static>> {
    let mut reader = std::io::Cursor::new(pem);
    if let Some(key) = rustls_pemfile::private_key(&mut reader)? {
        return Ok(key);
    }

    bail!("No supported private key found in PEM");
}

#[derive(Debug)]
struct PinnedServerCertVerifier {
    expected_fingerprint: Vec<u8>,
}

impl PinnedServerCertVerifier {
    fn new(fingerprint: &str) -> Result<Self> {
        let expected_fingerprint = parse_fingerprint(fingerprint)?;
        if expected_fingerprint.is_empty() {
            bail!("Pinned fingerprint is empty");
        }
        Ok(Self {
            expected_fingerprint,
        })
    }
}

impl ServerCertVerifier for PinnedServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let actual = Sha256::digest(end_entity.as_ref());
        if &actual[..] != self.expected_fingerprint.as_slice() {
            return Err(rustls::Error::General(
                "Server certificate fingerprint mismatch".into(),
            ));
        }

        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}

fn parse_fingerprint(fingerprint: &str) -> Result<Vec<u8>> {
    let value = fingerprint.trim();
    let hex_part = value
        .strip_prefix("SHA256:")
        .ok_or_else(|| anyhow!("Fingerprint must start with SHA256:"))?;

    let sanitized: String = hex_part.chars().filter(|c| c.is_ascii_hexdigit()).collect();

    let bytes =
        hex::decode(&sanitized).with_context(|| format!("Invalid fingerprint hex: {sanitized}"))?;
    Ok(bytes)
}

/// Write the fingerprint to disk when trust-on-first-use enrollment happens.
pub fn persist_fingerprint(cert_paths: &CertificatePaths, fingerprint: &str) -> Result<()> {
    fs::write(&cert_paths.server_fingerprint, fingerprint).with_context(|| {
        format!(
            "Failed to write fingerprint file {}",
            cert_paths.server_fingerprint.display()
        )
    })
}

/// Load the pinned fingerprint if it exists.
pub fn load_pinned_fingerprint(cert_paths: &CertificatePaths) -> Result<String> {
    let contents = fs::read_to_string(&cert_paths.server_fingerprint).with_context(|| {
        format!(
            "Failed to read fingerprint file {}",
            cert_paths.server_fingerprint.display()
        )
    })?;
    Ok(contents.trim().to_string())
}

/// Convenience helper that loads endpoint details from registry entry.
pub async fn connect_registered(
    entry: &ServerRegistryEntry,
    cert_paths: &CertificatePaths,
) -> Result<HandControlClient> {
    let address = entry
        .ip
        .as_ref()
        .or_else(|| entry.hostname.as_ref())
        .ok_or_else(|| anyhow!("Registry entry missing address"))?;
    let port = entry.port.unwrap_or(50051);

    let fingerprint =
        load_pinned_fingerprint(cert_paths).context("Failed to load pinned server fingerprint")?;

    HandControlClient::connect(address, port, cert_paths, &fingerprint).await
}
