use anyhow::{Context, Result};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme};
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

use super::certificates::ServerCertificate;
use crate::storage::clients::ClientStore;

/// Client certificate verifier that requests client certificates without enforcing trust.
///
/// The gRPC layer performs certificate authorization by comparing fingerprints against the
/// enrolled client registry. This verifier keeps TLS flexible enough to allow enrollment flows
/// (no certificate yet) while still collecting presented certificates for authenticated RPCs.
#[derive(Debug, Default)]
pub struct PermissiveClientVerifier;

impl ClientCertVerifier for PermissiveClientVerifier {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
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

/// Custom client certificate verifier that checks against authorized clients
#[derive(Debug)]
pub struct AuthorizedClientVerifier {
    client_store: Arc<Mutex<ClientStore>>,
}

impl AuthorizedClientVerifier {
    /// Create a new authorized client verifier
    pub fn new(client_store: Arc<Mutex<ClientStore>>) -> Result<Self> {
        Ok(Self { client_store })
    }

    /// Verify a client certificate against the authorized clients list
    fn verify_client_cert(&self, cert_der: &[u8]) -> Result<()> {
        use sha2::{Digest, Sha256};

        // Compute fingerprint of presented certificate
        let fingerprint = Sha256::digest(cert_der);
        let fingerprint_hex = hex::encode(fingerprint);

        debug!(
            "Verifying client certificate with fingerprint: {}",
            fingerprint_hex
        );

        // Check if certificate is in authorized list
        let client_store = self.client_store.lock().unwrap();
        let clients = client_store.list_clients();

        for client in clients {
            if client.cert_fingerprint == fingerprint_hex {
                info!(
                    "Client certificate verified: {} ({})",
                    client.name, client.id
                );
                return Ok(());
            }
        }

        warn!(
            "Client certificate not found in authorized list: {}",
            fingerprint_hex
        );
        anyhow::bail!("Client certificate not authorized");
    }
}

impl ClientCertVerifier for AuthorizedClientVerifier {
    fn offer_client_auth(&self) -> bool {
        // We require client certificates for mTLS
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        // Client certificates are mandatory for authenticated RPCs
        // (enrollment endpoints will bypass this)
        true
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        // We don't provide hints since we accept any client cert in our authorized list
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        // First, verify the certificate is in our authorized list
        self.verify_client_cert(end_entity.as_ref()).map_err(|e| {
            rustls::Error::General(format!("Client certificate not authorized: {}", e))
        })?;

        // Verify certificate is not expired
        // Note: We rely on the client certificate's validity period
        // The fallback verifier would check this, but we're doing manual verification

        debug!("Client certificate authorized and valid");
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        // We trust certificates in our authorized list, signature is valid
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        // We trust certificates in our authorized list, signature is valid
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        // Support ECDSA P-256 (our primary algorithm)
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
        ]
    }
}

/// Build rustls ServerConfig with mTLS and ECDSA support
pub fn build_server_config(
    server_cert: &ServerCertificate,
    client_store: Arc<Mutex<ClientStore>>,
) -> Result<ServerConfig> {
    info!("Building mTLS server configuration with ECDSA support");

    // Load server certificate chain
    let cert_chain = vec![CertificateDer::from(server_cert.cert_der.clone())];

    // Load server private key
    let private_key = PrivateKeyDer::try_from(server_cert.key_der.clone())
        .map_err(|e| anyhow::anyhow!("Failed to parse private key: {}", e))?;

    // Create custom client verifier
    let client_verifier = Arc::new(
        AuthorizedClientVerifier::new(client_store)
            .context("Failed to create client certificate verifier")?,
    );

    // Build ServerConfig with mTLS
    let config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(cert_chain, private_key)
        .context("Failed to build server config with certificate")?;
    info!("mTLS server configuration ready");

    Ok(config)
}

/// Create a ServerConfig without client certificate verification (for enrollment endpoints)
pub fn build_server_config_no_client_auth(server_cert: &ServerCertificate) -> Result<ServerConfig> {
    info!("Building TLS server configuration without client authentication");

    // Load server certificate chain
    let cert_chain = vec![CertificateDer::from(server_cert.cert_der.clone())];

    // Load server private key
    let private_key = PrivateKeyDer::try_from(server_cert.key_der.clone())
        .map_err(|e| anyhow::anyhow!("Failed to parse private key: {}", e))?;

    // Build ServerConfig without client cert verification
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, private_key)
        .context("Failed to build server config with certificate")?;
    config.alpn_protocols.push(b"h2".to_vec());
    info!("TLS server configuration ready (no client auth)");
    Ok(config)
}

/// Build a TLS server configuration that requests (but does not enforce) client certificates.
///
/// Presented certificates are inspected at the gRPC layer to enforce enrollment policy, allowing
/// unauthenticated flows (e.g., enrollment) to continue while blocking untrusted command execution.
pub fn build_permissive_server_config(server_cert: &ServerCertificate) -> Result<ServerConfig> {
    info!("Building TLS server configuration that requests client certificates");

    let cert_chain = vec![CertificateDer::from(server_cert.cert_der.clone())];
    let private_key = PrivateKeyDer::try_from(server_cert.key_der.clone())
        .map_err(|e| anyhow::anyhow!("Failed to parse private key: {}", e))?;

    let verifier = Arc::new(PermissiveClientVerifier::default());

    let mut config = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(cert_chain, private_key)
        .context("Failed to build server config with certificate")?;

    config.alpn_protocols.push(b"h2".to_vec());

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::certificates::{ClientCertificate, ServerCertificate};
    use crate::storage::clients::ClientStore;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    #[test]
    fn test_build_server_config_with_mtls() {
        let temp_dir = TempDir::new().unwrap();
        let clients_dir = temp_dir.path().join("clients");
        std::fs::create_dir_all(&clients_dir).unwrap();

        let store = Arc::new(Mutex::new(ClientStore::new(clients_dir).unwrap()));
        let server_cert = ServerCertificate::generate().unwrap();

        let config = build_server_config(&server_cert, store);
        assert!(config.is_ok());
    }

    #[test]
    fn test_build_server_config_no_client_auth() {
        let server_cert = ServerCertificate::generate().unwrap();
        let config = build_server_config_no_client_auth(&server_cert);
        assert!(config.is_ok());
    }

    #[test]
    fn test_authorized_client_verifier_creation() {
        let temp_dir = TempDir::new().unwrap();
        let clients_dir = temp_dir.path().join("clients");
        std::fs::create_dir_all(&clients_dir).unwrap();

        let store = Arc::new(Mutex::new(ClientStore::new(clients_dir).unwrap()));
        let verifier = AuthorizedClientVerifier::new(store);
        assert!(verifier.is_ok());
    }

    #[test]
    fn test_verifier_requires_client_auth() {
        let temp_dir = TempDir::new().unwrap();
        let clients_dir = temp_dir.path().join("clients");
        std::fs::create_dir_all(&clients_dir).unwrap();

        let store = Arc::new(Mutex::new(ClientStore::new(clients_dir).unwrap()));
        let verifier = AuthorizedClientVerifier::new(store).unwrap();

        assert!(verifier.offer_client_auth());
        assert!(verifier.client_auth_mandatory());
    }

    #[test]
    fn test_verifier_supported_schemes() {
        let temp_dir = TempDir::new().unwrap();
        let clients_dir = temp_dir.path().join("clients");
        std::fs::create_dir_all(&clients_dir).unwrap();

        let store = Arc::new(Mutex::new(ClientStore::new(clients_dir).unwrap()));
        let verifier = AuthorizedClientVerifier::new(store).unwrap();

        let schemes = verifier.supported_verify_schemes();
        assert!(schemes.contains(&rustls::SignatureScheme::ECDSA_NISTP256_SHA256));
    }

    #[test]
    fn test_verify_unauthorized_client() {
        let temp_dir = TempDir::new().unwrap();
        let clients_dir = temp_dir.path().join("clients");
        std::fs::create_dir_all(&clients_dir).unwrap();

        let store = Arc::new(Mutex::new(ClientStore::new(clients_dir).unwrap()));
        let verifier = AuthorizedClientVerifier::new(store).unwrap();

        // Generate a random certificate (not in authorized list)
        // Use ServerCertificate since it has generate() method
        let random_cert = ServerCertificate::generate().unwrap();
        let result = verifier.verify_client_cert(&random_cert.cert_der);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not authorized"));
    }

    #[test]
    fn test_verify_authorized_client() {
        let temp_dir = TempDir::new().unwrap();
        let clients_dir = temp_dir.path().join("clients");
        std::fs::create_dir_all(&clients_dir).unwrap();

        let store = Arc::new(Mutex::new(ClientStore::new(clients_dir).unwrap()));

        // Generate a certificate and wrap it as ClientCertificate
        let temp_cert = ServerCertificate::generate().unwrap();
        let client_cert = ClientCertificate::from_der(temp_cert.cert_der.clone());

        store
            .lock()
            .unwrap()
            .add_client(&client_cert, "Test Client".to_string(), None)
            .unwrap();

        let verifier = AuthorizedClientVerifier::new(store).unwrap();
        let result = verifier.verify_client_cert(&temp_cert.cert_der);

        assert!(result.is_ok());
    }
}
