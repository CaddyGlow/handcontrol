use anyhow::{Context, Result};
use rcgen::{CertificateParams, DistinguishedName, KeyPair, PKCS_ECDSA_P256_SHA256};
use rustls_pemfile::{certs, pkcs8_private_keys};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::BufReader;
use std::path::Path;
use time::{Duration, OffsetDateTime};
use tracing::{info, warn};

/// Server certificate and private key pair
#[derive(Clone)]
pub struct ServerCertificate {
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
    pub fingerprint: [u8; 32],
}

impl ServerCertificate {
    /// Generate a new self-signed ECDSA P-256 certificate
    pub fn generate() -> Result<Self> {
        info!("Generating new ECDSA P-256 server certificate");

        // Generate ECDSA P-256 key pair
        let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
            .context("Failed to generate ECDSA P-256 key pair")?;

        // Create certificate parameters
        let mut params = CertificateParams::default();

        // Set distinguished name
        let mut dn = DistinguishedName::new();
        dn.push(rcgen::DnType::CommonName, "HandControl Server");
        dn.push(rcgen::DnType::OrganizationName, "HandControl");
        params.distinguished_name = dn;

        // Set validity period (10 years)
        let now = OffsetDateTime::now_utc();
        params.not_before = now;
        params.not_after = now + Duration::days(3650); // 10 years

        // Generate self-signed certificate with ECDSA key
        let cert = params
            .self_signed(&key_pair)
            .context("Failed to generate self-signed certificate")?;

        let cert_der = cert.der().to_vec();
        let key_der = key_pair.serialize_der();

        // Compute SHA256 fingerprint
        let fingerprint = compute_fingerprint(&cert_der);

        info!(
            "Generated ECDSA P-256 certificate with fingerprint: {}",
            hex::encode(fingerprint)
        );

        Ok(Self {
            cert_der,
            key_der,
            fingerprint,
        })
    }

    /// Save certificate and key to PEM files
    pub fn save_to_files(&self, cert_path: &Path, key_path: &Path) -> Result<()> {
        info!("Saving certificate to {}", cert_path.display());
        info!("Saving private key to {}", key_path.display());

        // Convert to PEM format
        let cert_pem = pem::encode(&pem::Pem::new("CERTIFICATE", self.cert_der.clone()));
        let key_pem = pem::encode(&pem::Pem::new("PRIVATE KEY", self.key_der.clone()));

        // Write to files
        fs::write(cert_path, cert_pem)
            .with_context(|| format!("Failed to write certificate to {}", cert_path.display()))?;

        fs::write(key_path, key_pem)
            .with_context(|| format!("Failed to write private key to {}", key_path.display()))?;

        info!("Certificate and key saved successfully");
        Ok(())
    }

    /// Load certificate and key from PEM files
    pub fn load_from_files(cert_path: &Path, key_path: &Path) -> Result<Self> {
        info!("Loading certificate from {}", cert_path.display());
        info!("Loading private key from {}", key_path.display());

        // Read certificate
        let cert_file = fs::File::open(cert_path)
            .with_context(|| format!("Failed to open certificate file: {}", cert_path.display()))?;
        let mut cert_reader = BufReader::new(cert_file);
        let cert_ders = certs(&mut cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse certificate PEM")?;

        if cert_ders.is_empty() {
            anyhow::bail!("No certificates found in {}", cert_path.display());
        }

        let cert_der = cert_ders[0].to_vec();

        // Read private key
        let key_file = fs::File::open(key_path)
            .with_context(|| format!("Failed to open private key file: {}", key_path.display()))?;
        let mut key_reader = BufReader::new(key_file);
        let key_ders = pkcs8_private_keys(&mut key_reader)
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse private key PEM")?;

        if key_ders.is_empty() {
            anyhow::bail!("No private keys found in {}", key_path.display());
        }

        let key_der = key_ders[0].secret_pkcs8_der().to_vec();

        // Compute fingerprint
        let fingerprint = compute_fingerprint(&cert_der);

        info!(
            "Loaded ECDSA P-256 certificate with fingerprint: {}",
            hex::encode(fingerprint)
        );

        Ok(Self {
            cert_der,
            key_der,
            fingerprint,
        })
    }

    /// Get fingerprint as hex string
    pub fn fingerprint_hex(&self) -> String {
        hex::encode(self.fingerprint)
    }

    /// Get fingerprint in SHA256:hex format
    pub fn fingerprint_display(&self) -> String {
        format!("SHA256:{}", hex::encode(self.fingerprint))
    }
}

/// Compute SHA256 fingerprint of a certificate
pub fn compute_fingerprint(cert_der: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(cert_der);
    hasher.finalize().into()
}

/// Client certificate information
#[derive(Debug, Clone)]
pub struct ClientCertificate {
    pub cert_der: Vec<u8>,
    pub fingerprint: [u8; 32],
}

impl ClientCertificate {
    /// Create from DER-encoded certificate
    pub fn from_der(cert_der: Vec<u8>) -> Self {
        let fingerprint = compute_fingerprint(&cert_der);
        Self {
            cert_der,
            fingerprint,
        }
    }

    /// Get fingerprint as hex string
    pub fn fingerprint_hex(&self) -> String {
        hex::encode(self.fingerprint)
    }
}

/// Ensure server certificate exists, generate if not
pub fn ensure_server_certificate(cert_path: &Path, key_path: &Path) -> Result<ServerCertificate> {
    if cert_path.exists() && key_path.exists() {
        info!("Server certificate exists, loading from files");
        match ServerCertificate::load_from_files(cert_path, key_path) {
            Ok(cert) => return Ok(cert),
            Err(e) => {
                warn!(
                    "Failed to load existing certificate: {}. Generating new one.",
                    e
                );
            }
        }
    }

    info!("Server certificate does not exist, generating new one");
    let cert = ServerCertificate::generate()?;
    cert.save_to_files(cert_path, key_path)?;
    Ok(cert)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_generate_certificate() {
        let cert = ServerCertificate::generate().unwrap();
        assert!(!cert.cert_der.is_empty());
        assert!(!cert.key_der.is_empty());
        assert_eq!(cert.fingerprint.len(), 32);
    }

    #[test]
    fn test_save_and_load_certificate() {
        let temp_dir = TempDir::new().unwrap();
        let cert_path = temp_dir.path().join("server.crt");
        let key_path = temp_dir.path().join("server.key");

        // Generate and save
        let cert1 = ServerCertificate::generate().unwrap();
        cert1.save_to_files(&cert_path, &key_path).unwrap();

        // Load back
        let cert2 = ServerCertificate::load_from_files(&cert_path, &key_path).unwrap();

        // Should match
        assert_eq!(cert1.cert_der, cert2.cert_der);
        assert_eq!(cert1.key_der, cert2.key_der);
        assert_eq!(cert1.fingerprint, cert2.fingerprint);
    }

    #[test]
    fn test_fingerprint_formats() {
        let cert = ServerCertificate::generate().unwrap();
        let hex = cert.fingerprint_hex();
        let display = cert.fingerprint_display();

        assert_eq!(hex.len(), 64); // 32 bytes = 64 hex chars
        assert!(display.starts_with("SHA256:"));
    }

    #[test]
    fn test_ensure_certificate_generates_new() {
        let temp_dir = TempDir::new().unwrap();
        let cert_path = temp_dir.path().join("server.crt");
        let key_path = temp_dir.path().join("server.key");

        let _cert = ensure_server_certificate(&cert_path, &key_path).unwrap();
        assert!(cert_path.exists());
        assert!(key_path.exists());
    }

    #[test]
    fn test_ensure_certificate_loads_existing() {
        let temp_dir = TempDir::new().unwrap();
        let cert_path = temp_dir.path().join("server.crt");
        let key_path = temp_dir.path().join("server.key");

        // Generate first time
        let cert1 = ensure_server_certificate(&cert_path, &key_path).unwrap();

        // Should load existing
        let cert2 = ensure_server_certificate(&cert_path, &key_path).unwrap();

        // Should be the same certificate
        assert_eq!(cert1.fingerprint, cert2.fingerprint);
    }

    #[test]
    fn test_client_certificate_from_der() {
        let server_cert = ServerCertificate::generate().unwrap();
        let client_cert = ClientCertificate::from_der(server_cert.cert_der.clone());

        assert_eq!(client_cert.fingerprint, server_cert.fingerprint);
        assert_eq!(client_cert.cert_der, server_cert.cert_der);
    }
}
