use crate::config::parser::{DEFAULT_RELAY_SUBPROTOCOL, RelayConfig};
use crate::relay::tokens::TokenIssuer;
use anyhow::{Context, Result, anyhow, bail};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use rustls::DigitallySignedStruct;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{
        Message, client::IntoClientRequest, http::HeaderValue, protocol::frame::Payload,
    },
};
use tracing::{error, info, trace, warn};
use uuid::Uuid;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type LocalReadHalf = Box<dyn AsyncRead + Send + Unpin>;
type LocalWriteHalf = Box<dyn AsyncWrite + Send + Unpin>;

const CONTROL_PING_INTERVAL_SECS: u64 = 30;

#[derive(Debug, Clone, Copy)]
struct RelayTlsOptions {
    allow_self_signed: bool,
    pinned_cert_sha256: Option<[u8; 32]>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RelayClientState {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ControlMessage {
    #[serde(rename = "register_ack")]
    RegisterAck {
        status: String,
        retry_after_seconds: Option<u32>,
    },
    #[serde(rename = "open_tunnel")]
    OpenTunnel {
        tunnel_id: String,
        client_id: String,
        preferred_protocol: String,
        expires_at: u64,
        server_secret: String,
    },
    #[serde(rename = "ping")]
    Ping {},
}

#[derive(Debug, Serialize)]
struct TunnelReadyMessage {
    #[serde(rename = "type")]
    r#type: &'static str,
    tunnel_id: String,
    role: &'static str,
}

pub struct RelayClient {
    config: RelayConfig,
    server_id: Uuid,
    token_issuer: Arc<TokenIssuer>,
    local_grpc_endpoint: SocketAddr,
    state: Arc<RwLock<RelayClientState>>,
    tls_authority: String,
}

impl RelayClient {
    pub fn new(
        config: RelayConfig,
        server_id: Uuid,
        token_issuer: Arc<TokenIssuer>,
        local_grpc_endpoint: SocketAddr,
    ) -> Self {
        Self {
            config,
            server_id,
            token_issuer,
            local_grpc_endpoint,
            state: Arc::new(RwLock::new(RelayClientState::Disconnected)),
            tls_authority: format!("handcontrol.local:{}", local_grpc_endpoint.port()),
        }
    }

    fn tls_options(&self) -> Result<RelayTlsOptions> {
        Ok(RelayTlsOptions {
            allow_self_signed: self.config.allow_self_signed_tls,
            pinned_cert_sha256: parse_pinned_cert(self.config.pinned_cert_sha256.as_deref())?,
        })
    }

    fn relay_subprotocol(&self) -> &str {
        let configured = self.config.websocket_subprotocol.trim();
        if configured.is_empty() {
            DEFAULT_RELAY_SUBPROTOCOL
        } else {
            &self.config.websocket_subprotocol
        }
    }

    pub async fn get_state(&self) -> RelayClientState {
        self.state.read().await.clone()
    }

    /// Start the relay client with automatic reconnection
    pub async fn start(self: Arc<Self>) {
        let mut reconnect_delay = 1u64;
        let max_delay = self.config.reconnect_delay_seconds;

        loop {
            *self.state.write().await = RelayClientState::Connecting;

            let mut reset_backoff = false;

            match self.connect_and_run().await {
                Ok(_) => {
                    info!("Relay connection closed gracefully");
                    reset_backoff = true;
                    *self.state.write().await = RelayClientState::Disconnected;
                }
                Err(e) => {
                    error!("Relay connection failed: {:#}", e);
                    let was_connected = {
                        let state = self.state.read().await;
                        matches!(*state, RelayClientState::Connected)
                    };
                    {
                        let mut state = self.state.write().await;
                        *state = RelayClientState::Error(e.to_string());
                    }
                    reset_backoff = was_connected;
                }
            }

            if !self.config.auto_connect {
                info!("Auto-reconnect disabled, stopping relay client");
                break;
            }

            if reset_backoff {
                reconnect_delay = 1;
            }

            info!("Reconnecting to relay in {} seconds", reconnect_delay);
            sleep(Duration::from_secs(reconnect_delay)).await;

            if reset_backoff {
                reconnect_delay = 1;
            } else {
                reconnect_delay = (reconnect_delay * 2).min(max_delay);
            }
        }
    }

    /// Connect to the relay server and maintain the control channel
    async fn connect_and_run(&self) -> Result<()> {
        let relay_url = self
            .config
            .relay_server_url
            .as_ref()
            .context("relay_server_url not configured")?;

        let register_url = format!("{}/register", relay_url);
        info!(url = %register_url, "Connecting to relay server");

        let tls_options = self.tls_options()?;

        let subprotocol = self.relay_subprotocol();

        let (ws_stream, _) = if register_url.starts_with("wss://") {
            if let Some(connector) = build_tls_connector(tls_options)? {
                tokio_tungstenite::connect_async_tls_with_config(
                    build_relay_request(&register_url, subprotocol)?,
                    None,
                    false,
                    Some(connector),
                )
                .await
                .context("Failed to connect to relay server")?
            } else {
                connect_async(build_relay_request(&register_url, subprotocol)?)
                    .await
                    .context("Failed to connect to relay server")?
            }
        } else {
            connect_async(build_relay_request(&register_url, subprotocol)?)
                .await
                .context("Failed to connect to relay server")?
        };

        let (sink, mut stream) = ws_stream.split();
        let sink = Arc::new(Mutex::new(sink));

        // Send register message
        let relay_secret = self
            .config
            .relay_auth_secret
            .as_ref()
            .context("relay_auth_secret not configured")?;

        let public_key = self.token_issuer.public_key_base64();

        let register_msg = serde_json::json!({
            "type": "register",
            "server_id": self.server_id.to_string(),
            "relay_secret": relay_secret,
            "server_version": env!("CARGO_PKG_VERSION"),
            "capabilities": ["relay.v1"],
            "public_key": public_key,
            "max_tunnels": self.config.max_relay_tunnels.unwrap_or(10),
            "tls_authority": self.tls_authority,
        });

        {
            let mut sink_guard = sink.lock().await;
            sink_guard
                .send(Message::Text(register_msg.to_string().into()))
                .await
                .context("Failed to send register message")?;
        }

        // Wait for register_ack
        match stream.next().await {
            Some(Ok(Message::Text(payload))) => {
                let msg: ControlMessage = serde_json::from_str(payload.as_str())
                    .context("Failed to parse register_ack")?;

                match msg {
                    ControlMessage::RegisterAck { status, .. } => {
                        if status != "ok" {
                            return Err(anyhow!("Relay registration failed: {}", status));
                        }
                        info!("Successfully registered with relay server");
                    }
                    _ => return Err(anyhow!("Expected register_ack, got different message")),
                }
            }
            Some(Ok(msg)) => return Err(anyhow!("Expected text message, got: {:?}", msg)),
            Some(Err(e)) => return Err(anyhow!("WebSocket error: {}", e)),
            None => return Err(anyhow!("Connection closed before register_ack")),
        }

        *self.state.write().await = RelayClientState::Connected;

        let keepalive_handle = self.spawn_control_keepalive(sink.clone());

        // Listen for control messages
        let result = self.listen_control_channel(&mut stream).await;

        keepalive_handle.abort();
        let _ = keepalive_handle.await;

        result
    }

    async fn listen_control_channel(
        &self,
        stream: &mut futures_util::stream::SplitStream<WsStream>,
    ) -> Result<()> {
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(Message::Text(payload)) => {
                    if let Err(e) = self.handle_control_message(payload.as_str()).await {
                        warn!("Failed to handle control message: {:#}", e);
                    }
                }
                Ok(Message::Close(_)) => {
                    info!("Relay server closed connection");
                    break;
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {
                    // WebSocket library handles these automatically
                }
                Ok(other) => {
                    warn!("Unexpected WebSocket message type: {:?}", other);
                }
                Err(e) => {
                    return Err(anyhow!("WebSocket error: {}", e));
                }
            }
        }

        Ok(())
    }

    fn spawn_control_keepalive(
        &self,
        sink: Arc<Mutex<SplitSink<WsStream, Message>>>,
    ) -> JoinHandle<()> {
        let server_id = self.server_id;
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(CONTROL_PING_INTERVAL_SECS));
            loop {
                ticker.tick().await;

                let mut guard = sink.lock().await;
                match guard.send(Message::Ping(Vec::new().into())).await {
                    Ok(_) => {
                        trace!(%server_id, "Sent relay control ping");
                    }
                    Err(err) => {
                        warn!(%server_id, "Relay control ping failed: {err}");
                        break;
                    }
                }
            }
        })
    }

    async fn handle_control_message(&self, payload: &str) -> Result<()> {
        let msg: ControlMessage =
            serde_json::from_str(payload).context("Failed to parse control message")?;

        match msg {
            ControlMessage::OpenTunnel {
                tunnel_id,
                client_id,
                server_secret,
                ..
            } => {
                info!(
                    "Relay requested tunnel {} for client {}",
                    tunnel_id, client_id
                );
                self.spawn_tunnel_task(&tunnel_id, &server_secret).await?;
            }
            ControlMessage::Ping {} => {
                // Pong is handled by WebSocket library
            }
            other => {
                warn!("Unexpected control message: {:?}", other);
            }
        }

        Ok(())
    }

    async fn spawn_tunnel_task(&self, tunnel_id: &str, server_secret: &str) -> Result<()> {
        let relay_url = self
            .config
            .relay_server_url
            .as_ref()
            .context("relay_server_url not configured")?;

        let encoded_secret: String =
            url::form_urlencoded::byte_serialize(server_secret.as_bytes()).collect();
        let tunnel_url = format!(
            "{}/tunnel/{}?role=server&token={}",
            relay_url, tunnel_id, encoded_secret
        );
        let local_endpoint = self.local_grpc_endpoint;
        let tunnel_id = tunnel_id.to_string();
        let tls_options = self.tls_options()?;

        let subprotocol = self.relay_subprotocol().to_string();

        tokio::spawn(async move {
            if let Err(e) = handle_tunnel(
                tunnel_url,
                tunnel_id,
                local_endpoint,
                tls_options,
                subprotocol,
            )
            .await
            {
                error!("Tunnel task failed: {:#}", e);
            }
        });

        Ok(())
    }
}

/// Handle a single tunnel connection by bridging WebSocket and local TCP
async fn handle_tunnel(
    tunnel_url: String,
    tunnel_id: String,
    local_endpoint: SocketAddr,
    tls_options: RelayTlsOptions,
    subprotocol: String,
) -> Result<()> {
    info!(tunnel_id = %tunnel_id, "Opening tunnel to relay");

    // Connect to relay tunnel endpoint
    let (ws_stream, _) = if tunnel_url.starts_with("wss://") {
        if let Some(connector) = build_tls_connector(tls_options)? {
            tokio_tungstenite::connect_async_tls_with_config(
                build_relay_request(&tunnel_url, &subprotocol)?,
                None,
                false,
                Some(connector),
            )
            .await
            .context("Failed to connect to tunnel endpoint")?
        } else {
            connect_async(build_relay_request(&tunnel_url, &subprotocol)?)
                .await
                .context("Failed to connect to tunnel endpoint")?
        }
    } else {
        connect_async(build_relay_request(&tunnel_url, &subprotocol)?)
            .await
            .context("Failed to connect to tunnel endpoint")?
    };

    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    // Send tunnel_ready
    let ready_msg = TunnelReadyMessage {
        r#type: "tunnel_ready",
        tunnel_id: tunnel_id.clone(),
        role: "server",
    };

    ws_sink
        .send(Message::Text(serde_json::to_string(&ready_msg)?.into()))
        .await
        .context("Failed to send tunnel_ready")?;

    info!(tunnel_id = %tunnel_id, "Tunnel ready, connecting to local gRPC");

    let tcp_stream = TcpStream::connect(local_endpoint)
        .await
        .context("Failed to connect to local gRPC server")?;
    let (read_half, write_half) = tcp_stream.into_split();
    let mut local_read: LocalReadHalf = Box::new(read_half);
    let mut local_write: LocalWriteHalf = Box::new(write_half);

    info!(tunnel_id = %tunnel_id, "Bridge established, forwarding data");

    // Task 1: WebSocket -> Local TCP
    let ws_to_local = async {
        while let Some(msg) = ws_stream.next().await {
            match msg {
                Ok(Message::Binary(payload)) => {
                    local_write
                        .write_all(payload.as_slice())
                        .await
                        .context("Failed to write to local TCP")?;
                }
                Ok(Message::Close(_)) => {
                    break;
                }
                Err(e) => {
                    return Err(anyhow!("WebSocket read error: {}", e));
                }
                _ => {} // Ignore other message types
            }
        }
        Ok::<_, anyhow::Error>(())
    };

    // Task 2: Local TCP -> WebSocket
    let local_to_ws = async {
        let mut buffer = vec![0u8; 8192];
        loop {
            match local_read.read(&mut buffer).await {
                Ok(0) => break, // EOF
                Ok(n) => {
                    ws_sink
                        .send(Message::Binary(Payload::Vec(buffer[..n].to_vec())))
                        .await
                        .context("Failed to send to WebSocket")?;
                }
                Err(e) => {
                    return Err(anyhow!("Local TCP read error: {}", e));
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    };

    // Run both directions concurrently
    tokio::select! {
        result = ws_to_local => result?,
        result = local_to_ws => result?,
    }

    info!(tunnel_id = %tunnel_id, "Tunnel closed");
    Ok(())
}

fn build_relay_request(
    url: &str,
    subprotocol: &str,
) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request> {
    let mut request = url
        .into_client_request()
        .context("Failed to construct relay WebSocket request")?;
    let header = HeaderValue::from_str(subprotocol)
        .context("relay.websocket_subprotocol must be a valid header value")?;
    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", header);
    Ok(request)
}

fn parse_pinned_cert(value: Option<&str>) -> Result<Option<[u8; 32]>> {
    if let Some(fingerprint) = value {
        let normalized: String = fingerprint
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != ':')
            .collect();

        if normalized.is_empty() {
            bail!("relay.pinned_cert_sha256 must not be empty");
        }

        let bytes = hex::decode(&normalized)
            .context("relay.pinned_cert_sha256 must be valid hexadecimal")?;

        if bytes.len() != 32 {
            bail!("relay.pinned_cert_sha256 must decode to 32 bytes (SHA-256)");
        }

        let mut fingerprint_bytes = [0u8; 32];
        fingerprint_bytes.copy_from_slice(&bytes);
        Ok(Some(fingerprint_bytes))
    } else {
        Ok(None)
    }
}

fn build_tls_connector(tls_options: RelayTlsOptions) -> Result<Option<Connector>> {
    if !tls_options.allow_self_signed {
        return Ok(None);
    }

    #[derive(Debug)]
    struct AcceptSelfSignedVerifier {
        pinned_cert_sha256: Option<[u8; 32]>,
    }

    impl ServerCertVerifier for AcceptSelfSignedVerifier {
        fn verify_server_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            if let Some(expected) = self.pinned_cert_sha256 {
                let digest = Sha256::digest(end_entity.as_ref());
                let digest_bytes: &[u8] = digest.as_ref();
                let expected_bytes: &[u8] = expected.as_ref();
                if digest_bytes != expected_bytes {
                    return Err(rustls::Error::InvalidCertificate(
                        rustls::CertificateError::ApplicationVerificationFailure,
                    ));
                }
            }

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

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            vec![
                rustls::SignatureScheme::RSA_PKCS1_SHA256,
                rustls::SignatureScheme::RSA_PKCS1_SHA384,
                rustls::SignatureScheme::RSA_PKCS1_SHA512,
                rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
                rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
                rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
                rustls::SignatureScheme::RSA_PSS_SHA256,
                rustls::SignatureScheme::RSA_PSS_SHA384,
                rustls::SignatureScheme::RSA_PSS_SHA512,
                rustls::SignatureScheme::ED25519,
            ]
        }
    }

    let mut client_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(AcceptSelfSignedVerifier {
            pinned_cert_sha256: tls_options.pinned_cert_sha256,
        }))
        .with_no_client_auth();

    client_config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok(Some(Connector::Rustls(std::sync::Arc::new(client_config))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relay_client_state() {
        let state = RelayClientState::Connected;
        assert_eq!(state, RelayClientState::Connected);

        let error_state = RelayClientState::Error("test error".to_string());
        assert!(matches!(error_state, RelayClientState::Error(_)));
    }

    #[test]
    fn test_parse_pinned_cert_valid() {
        let fingerprint = (0..32).map(|_| "AA").collect::<Vec<_>>().join(":");

        let parsed = parse_pinned_cert(Some(&fingerprint)).expect("should parse");
        assert_eq!(parsed, Some([0xAA; 32]));
    }

    #[test]
    fn test_parse_pinned_cert_invalid_length() {
        let err = parse_pinned_cert(Some("DEADBEEF")).unwrap_err();
        assert!(
            err.to_string()
                .contains("relay.pinned_cert_sha256 must decode to 32 bytes")
        );
    }
}
