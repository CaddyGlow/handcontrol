use crate::config::parser::{DEFAULT_RELAY_SUBPROTOCOL, RelayConfig};
use crate::relay::tokens::TokenIssuer;
use crate::relay::transport::{
    ControlConnectParams, ControlFrame, ControlSession, ControlSink, RelayTransport,
    TunnelConnectParams, TlsOptions,
};
use crate::relay::transport::websocket::WebSocketTransport;
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep};
use tracing::{error, info, trace, warn};
use uuid::Uuid;

const CONTROL_PING_INTERVAL_SECS: u64 = 30;

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

pub struct RelayClient {
    config: RelayConfig,
    server_id: Uuid,
    token_issuer: Arc<TokenIssuer>,
    local_grpc_endpoint: SocketAddr,
    state: Arc<RwLock<RelayClientState>>,
    tls_authority: String,
    transport: Arc<dyn RelayTransport>,
}

impl RelayClient {
    pub fn new(
        config: RelayConfig,
        server_id: Uuid,
        token_issuer: Arc<TokenIssuer>,
        local_grpc_endpoint: SocketAddr,
    ) -> Self {
        let transport: Arc<dyn RelayTransport> =
            Arc::new(WebSocketTransport::default());

        Self {
            config,
            server_id,
            token_issuer,
            local_grpc_endpoint,
            state: Arc::new(RwLock::new(RelayClientState::Disconnected)),
            tls_authority: format!("handcontrol.local:{}", local_grpc_endpoint.port()),
            transport,
        }
    }

    pub fn with_transport(mut self, transport: Arc<dyn RelayTransport>) -> Self {
        self.transport = transport;
        self
    }

    fn tls_options(&self) -> Result<TlsOptions> {
        Ok(TlsOptions {
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
        let subprotocol = self.relay_subprotocol().to_string();

        let mut control_session = self
            .transport
            .connect_control(ControlConnectParams {
                register_url: register_url.clone(),
                subprotocol,
                tls: tls_options,
            })
            .await?;

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
        })
        .to_string();

        control_session
            .sink()
            .send_text(&register_msg)
            .await
            .context("Failed to send register message")?;

        // Wait for register_ack
        match control_session.next().await {
            Some(Ok(ControlFrame::Text(payload))) => {
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
            Some(Ok(other)) => {
                return Err(anyhow!("Expected text message, got: {:?}", other));
            }
            Some(Err(err)) => return Err(err),
            None => return Err(anyhow!("Connection closed before register_ack")),
        }

        *self.state.write().await = RelayClientState::Connected;

        let keepalive_handle = self.spawn_control_keepalive(control_session.sink());

        // Listen for control messages
        let result = self.listen_control_channel(&mut control_session).await;

        keepalive_handle.abort();
        let _ = keepalive_handle.await;

        result
    }

    async fn listen_control_channel(
        &self,
        session: &mut ControlSession,
    ) -> Result<()> {
        while let Some(frame) = session.next().await {
            match frame {
                Ok(ControlFrame::Text(payload)) => {
                    if let Err(e) = self.handle_control_message(payload.as_str()).await {
                        warn!("Failed to handle control message: {:#}", e);
                    }
                }
                Ok(ControlFrame::Close) => {
                    info!("Relay server closed connection");
                    break;
                }
                Ok(ControlFrame::Ping(_)) | Ok(ControlFrame::Pong(_)) => {
                    // Control session handles pings/pongs internally
                }
                Ok(other) => {
                    warn!("Unexpected control frame: {:?}", other);
                }
                Err(err) => return Err(err),
            }
        }

        Ok(())
    }

    fn spawn_control_keepalive(
        &self,
        sink: Arc<dyn ControlSink>,
    ) -> JoinHandle<()> {
        let server_id = self.server_id;
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(CONTROL_PING_INTERVAL_SECS));
            loop {
                ticker.tick().await;

                if let Err(err) = sink.send_ping().await {
                    warn!(%server_id, "Relay control ping failed: {err}");
                    break;
                }
                trace!(%server_id, "Sent relay control ping");
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

        self.transport
            .spawn_tunnel(TunnelConnectParams {
                tunnel_url,
                tunnel_id,
                subprotocol,
                local_endpoint,
                tls: tls_options,
            })
            .await
    }
}

/// Handle a single tunnel connection by bridging WebSocket and local TCP
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
