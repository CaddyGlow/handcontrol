use crate::config::parser::RelayConfig;
use crate::relay::tokens::TokenIssuer;
use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, protocol::frame::Payload},
    MaybeTlsStream,
    WebSocketStream
};
use tracing::{error, info, warn};
use uuid::Uuid;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

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

            match self.connect_and_run().await {
                Ok(_) => {
                    info!("Relay connection closed gracefully");
                    reconnect_delay = 1;
                }
                Err(e) => {
                    error!("Relay connection failed: {:#}", e);
                    *self.state.write().await = RelayClientState::Error(e.to_string());
                }
            }

            if !self.config.auto_connect {
                info!("Auto-reconnect disabled, stopping relay client");
                break;
            }

            info!("Reconnecting to relay in {} seconds", reconnect_delay);
            sleep(Duration::from_secs(reconnect_delay)).await;

            reconnect_delay = (reconnect_delay * 2).min(max_delay);
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

        let (ws_stream, _) = connect_async(&register_url)
            .await
            .context("Failed to connect to relay server")?;

        let (mut sink, mut stream) = ws_stream.split();

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
        });

        sink.send(Message::Text(register_msg.to_string().into()))
            .await
            .context("Failed to send register message")?;

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

        // Listen for control messages
        self.listen_control_channel(&mut stream).await
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

    async fn handle_control_message(&self, payload: &str) -> Result<()> {
        let msg: ControlMessage = serde_json::from_str(payload)
            .context("Failed to parse control message")?;

        match msg {
            ControlMessage::OpenTunnel {
                tunnel_id,
                client_id,
                ..
            } => {
                info!("Relay requested tunnel {} for client {}", tunnel_id, client_id);
                self.spawn_tunnel_task(&tunnel_id).await?;
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

    async fn spawn_tunnel_task(&self, tunnel_id: &str) -> Result<()> {
        let relay_url = self
            .config
            .relay_server_url
            .as_ref()
            .context("relay_server_url not configured")?;

        let tunnel_url = format!("{}/tunnel/{}?role=server", relay_url, tunnel_id);
        let local_endpoint = self.local_grpc_endpoint;
        let tunnel_id = tunnel_id.to_string();

        tokio::spawn(async move {
            if let Err(e) = handle_tunnel(tunnel_url, tunnel_id, local_endpoint).await {
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
) -> Result<()> {
    info!(tunnel_id = %tunnel_id, "Opening tunnel to relay");

    // Connect to relay tunnel endpoint
    let (ws_stream, _) = connect_async(&tunnel_url)
        .await
        .context("Failed to connect to tunnel endpoint")?;

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

    // Connect to local gRPC server
    let mut local_stream = TcpStream::connect(local_endpoint)
        .await
        .context("Failed to connect to local gRPC server")?;

    info!(tunnel_id = %tunnel_id, "Bridge established, forwarding data");

    // Bridge WebSocket <-> local TCP
    let (mut local_read, mut local_write) = local_stream.split();

    // Task 1: WebSocket -> Local TCP
    let ws_to_local = async {
        while let Some(msg) = ws_stream.next().await {
            match msg {
                Ok(Message::Binary(Payload::Vec(data))) => {
                    local_write
                        .write_all(&data)
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
}
