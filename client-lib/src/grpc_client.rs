use crate::{
    certificates::CertificatePaths, proto::remote_control_client::RemoteControlClient,
    storage::ServerRegistryEntry,
};
use anyhow::{anyhow, bail, Context, Result};
use hyper_util::rt::TokioIo;
use http::Uri;
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime},
    ClientConfig, SignatureScheme,
};
use sha2::{Digest, Sha256};
use std::{fs, sync::Arc};
use tokio::{
    net::TcpStream,
    time::{timeout, Duration},
};
use tokio_rustls::TlsConnector;
use tonic::transport::{Channel, Endpoint};
use tonic::Request;
use tower::service_fn;

const DEFAULT_CONNECT_TIMEOUT_SECONDS: u64 = 5;
const HANDSHAKE_TIMEOUT_SECONDS: u64 = 10;

/// gRPC client wrapper that handles TLS setup and exposes RPC helpers.
pub struct HandControlClient {
    inner: RemoteControlClient<Channel>,
}

impl HandControlClient {
    /// Establish a TLS-authenticated gRPC connection using stored credentials.
    pub async fn connect(
        address: &str,
        port: u16,
        cert_paths: &CertificatePaths,
        server_fingerprint: &str,
    ) -> Result<Self> {
        let tls_config = build_tls_config(cert_paths, server_fingerprint)
            .context("Failed to construct TLS configuration")?;

        let (endpoint_uri, socket_addr, server_name) = build_endpoints(address, port)?;

        let endpoint = Endpoint::from_shared(endpoint_uri.clone())
            .with_context(|| format!("Invalid endpoint URI: {endpoint_uri}"))?
            .connect_timeout(Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECONDS));

        let tls_connector = TlsConnector::from(tls_config.clone());
        let connector = service_fn(move |_: Uri| {
            let address = socket_addr.clone();
            let server_name = server_name.clone();
            let tls_connector = tls_connector.clone();
            async move {
                let tcp = timeout(
                    Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECONDS),
                    TcpStream::connect(address.as_str()),
                )
                .await
                .map_err(|_| anyhow!("TCP connect timed out"))??;

                let tls_stream = timeout(
                    Duration::from_secs(HANDSHAKE_TIMEOUT_SECONDS),
                    tls_connector.connect(server_name.clone(), tcp),
                )
                .await
                .map_err(|_| anyhow!("TLS handshake timed out"))??;

                Ok::<_, anyhow::Error>(TokioIo::new(tls_stream))
            }
        });

        let channel = endpoint
            .connect_with_connector(connector)
            .await
            .context("Failed to establish gRPC channel")?;

        let inner = RemoteControlClient::new(channel);
        Ok(Self { inner })
    }

    pub fn into_inner(self) -> RemoteControlClient<Channel> {
        self.inner
    }

    pub fn inner(&mut self) -> &mut RemoteControlClient<Channel> {
        &mut self.inner
    }

    pub async fn get_server_info(&mut self) -> Result<crate::proto::ServerInfoResponse> {
        let response = self
            .inner
            .get_server_info(Request::new(crate::proto::ServerInfoRequest {}))
            .await?
            .into_inner();

        Ok(response)
    }
}

fn build_endpoints(address: &str, port: u16) -> Result<(String, String, ServerName<'static>)> {
    let domain = normalize_domain(address);
    let uri = if domain.contains(':') {
        format!("https://[{domain}]:{port}")
    } else {
        format!("https://{domain}:{port}")
    };

    let socket = if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    };

    let server_name = ServerName::try_from(domain.clone())?;

    Ok((uri, socket, server_name))
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

fn normalize_domain(address: &str) -> String {
    address.trim_matches(|c| c == '[' || c == ']').to_string()
}
