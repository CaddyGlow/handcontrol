use crate::{
    certificates::CertificatePaths, proto::remote_control_client::RemoteControlClient,
    storage::ServerRegistryEntry,
};
use anyhow::{anyhow, bail, Context, Result};
use http::Uri;
use hyper_util::rt::TokioIo;
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime},
    ClientConfig, SignatureScheme,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
};
use tokio::{
    net::TcpStream,
    time::{timeout, Duration},
};
use tokio_rustls::TlsConnector;
use tonic::transport::{Channel, Endpoint};
use tonic::Request;
use tower::service_fn;
use uuid::Uuid;

const DEFAULT_CONNECT_TIMEOUT_SECONDS: u64 = 5;
const HANDSHAKE_TIMEOUT_SECONDS: u64 = 10;

/// gRPC client wrapper that handles TLS setup and exposes RPC helpers.
pub struct HandControlClient {
    inner: RemoteControlClient<Channel>,
}

impl HandControlClient {
    pub async fn connect(
        address: &str,
        port: u16,
        cert_paths: &CertificatePaths,
        server_fingerprint: &str,
    ) -> Result<Self> {
        let (cert_pem, key_pem) = cert_paths
            .load_client_credentials()
            .context("Failed to load stored client credentials")?;
        let cert_chain = load_certs(&cert_pem)?;
        let client_key = load_private_key(&key_pem)?;

        let result = connect_channel(
            address,
            port,
            Some(server_fingerprint),
            Some((cert_chain, client_key)),
        )
        .await?;

        Ok(Self {
            inner: RemoteControlClient::new(result.channel),
        })
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

/// Connect to a server without presenting client credentials when a fingerprint is already pinned.
pub async fn connect_unauthenticated(
    address: &str,
    port: u16,
    server_fingerprint: &str,
) -> Result<RemoteControlClient<Channel>> {
    let result = connect_channel(address, port, Some(server_fingerprint), None).await?;
    Ok(RemoteControlClient::new(result.channel))
}

/// Connect without prior fingerprint knowledge. Returns the negotiated fingerprint bytes.
pub async fn connect_unverified(
    address: &str,
    port: u16,
) -> Result<(RemoteControlClient<Channel>, Vec<u8>)> {
    let result = connect_channel(address, port, None, None).await?;
    Ok((RemoteControlClient::new(result.channel), result.fingerprint))
}

async fn connect_channel(
    address: &str,
    port: u16,
    expected_fingerprint: Option<&str>,
    client_auth: Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>,
) -> Result<ConnectResult> {
    let (tls_config, recorder) = build_tls_config(expected_fingerprint, client_auth)?;
    let (endpoint_uri, socket_addr, server_name) = build_endpoints(address, port)?;

    let endpoint = Endpoint::from_shared(endpoint_uri.clone())
        .with_context(|| format!("Invalid endpoint URI: {endpoint_uri}"))?
        .connect_timeout(Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECONDS));

    let tls_connector = TlsConnector::from(tls_config);
    let connector = service_fn(move |_: Uri| {
        let address = socket_addr;
        let server_name = server_name.clone();
        let tls_connector = tls_connector.clone();
        async move {
            let tcp = timeout(
                Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECONDS),
                TcpStream::connect(address),
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

    let fingerprint = recorder
        .fingerprint()
        .ok_or_else(|| anyhow!("Server fingerprint not captured during handshake"))?;

    Ok(ConnectResult {
        channel,
        fingerprint,
    })
}

fn build_endpoints(address: &str, port: u16) -> Result<(String, SocketAddr, ServerName<'static>)> {
    let domain = normalize_domain(address);
    let uri = if domain.contains(':') {
        format!("http://[{domain}]:{port}")
    } else {
        format!("http://{domain}:{port}")
    };

    let socket_literal = if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    };
    let socket_addr: SocketAddr = socket_literal
        .parse()
        .with_context(|| format!("Invalid socket address: {socket_literal}"))?;

    let server_name = if let Ok(ip) = domain.parse::<IpAddr>() {
        ServerName::IpAddress(ip.into())
    } else {
        ServerName::try_from(domain.as_str())?.to_owned()
    };

    Ok((uri, socket_addr, server_name))
}

fn build_tls_config(
    expected_fingerprint: Option<&str>,
    client_auth: Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>,
) -> Result<(Arc<ClientConfig>, Arc<dyn ServerCertRecorder>)> {
    let recorder: Arc<dyn ServerCertRecorder> = match expected_fingerprint {
        Some(value) => Arc::new(PinnedServerCertVerifier::new(value.to_string())?) as Arc<_>,
        None => Arc::new(RecordingServerCertVerifier::default()) as Arc<_>,
    };

    let builder = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(recorder.clone());

    let mut config = match client_auth {
        Some((cert_chain, key)) => builder.with_client_auth_cert(cert_chain, key)?,
        None => builder.with_no_client_auth(),
    };

    config.alpn_protocols.push(b"h2".to_vec());
    Ok((Arc::new(config), recorder))
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
struct ConnectResult {
    channel: Channel,
    fingerprint: Vec<u8>,
}

trait ServerCertRecorder: ServerCertVerifier + Send + Sync {
    fn fingerprint(&self) -> Option<Vec<u8>>;
}

#[derive(Debug)]
struct PinnedServerCertVerifier {
    expected_fingerprint: Vec<u8>,
    captured: Mutex<Option<Vec<u8>>>,
}

impl PinnedServerCertVerifier {
    fn new(fingerprint: String) -> Result<Self> {
        let expected_fingerprint = parse_fingerprint(&fingerprint)?;
        if expected_fingerprint.is_empty() {
            bail!("Pinned fingerprint is empty");
        }
        Ok(Self {
            expected_fingerprint,
            captured: Mutex::new(None),
        })
    }
}

impl ServerCertRecorder for PinnedServerCertVerifier {
    fn fingerprint(&self) -> Option<Vec<u8>> {
        self.captured.lock().ok().and_then(|guard| guard.clone())
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
        if let Ok(mut guard) = self.captured.lock() {
            *guard = Some(actual.to_vec());
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

#[derive(Default, Debug)]
struct RecordingServerCertVerifier {
    captured: Mutex<Option<Vec<u8>>>,
}

impl ServerCertRecorder for RecordingServerCertVerifier {
    fn fingerprint(&self) -> Option<Vec<u8>> {
        self.captured.lock().ok().and_then(|guard| guard.clone())
    }
}

impl ServerCertVerifier for RecordingServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let actual = Sha256::digest(end_entity.as_ref()).to_vec();
        if let Ok(mut guard) = self.captured.lock() {
            *guard = Some(actual);
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

/// Result of retrieving server information over an unauthenticated connection.
pub struct ServerInfoData {
    pub address: String,
    pub port: u16,
    pub server_id: Uuid,
    pub hostname: String,
    pub version: String,
    pub os: String,
    pub fingerprint: String,
}

/// Attempt to fetch server information from the provided addresses.
pub async fn fetch_server_info(
    addresses: &[String],
    port: u16,
    server_id_hint: Option<Uuid>,
) -> Result<ServerInfoData> {
    if addresses.is_empty() {
        bail!("No addresses provided to fetch server info");
    }

    let mut last_err: Option<anyhow::Error> = None;
    for address in addresses {
        match connect_unverified(address, port).await {
            Ok((mut client, fingerprint_bytes)) => {
                let response = client
                    .get_server_info(Request::new(crate::proto::ServerInfoRequest {}))
                    .await
                    .context("GetServerInfo RPC failed")?
                    .into_inner();

                let server_id = Uuid::parse_str(&response.server_id)
                    .context("Server returned invalid server_id")?;

                if let Some(expected) = server_id_hint {
                    if expected != server_id {
                        bail!(
                            "Server ID mismatch (expected {}, got {})",
                            expected,
                            server_id
                        );
                    }
                }

                let fingerprint = format!("SHA256:{}", hex::encode(&fingerprint_bytes));

                return Ok(ServerInfoData {
                    address: address.clone(),
                    port,
                    server_id,
                    hostname: response.hostname,
                    version: response.version,
                    os: response.os,
                    fingerprint,
                });
            }
            Err(err) => {
                last_err = Some(err);
                continue;
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow!("Failed to connect to any provided address")))
}
