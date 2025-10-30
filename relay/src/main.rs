mod config;

use crate::config::{default_config_path, load_config, RelayConfig};
use anyhow::{anyhow, Context, Result};
use axum::{
    Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use clap::{Parser, ValueHint};
use ed25519_dalek::{pkcs8::EncodePublicKey, VerifyingKey};
use handcontrol_relay::tunnel::state::TunnelState;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::sleep;
use tungstenite::protocol::frame::coding::CloseCode;
use tracing::{info, warn};
use uuid::Uuid;
use base64::{engine::general_purpose::STANDARD as Base64, Engine};

#[derive(Parser)]
#[command(name = "handcontrol-relay")]
struct Cli {
    /// Path to relay configuration file
    #[arg(long, value_hint = ValueHint::FilePath)]
    config: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt::init();

    let config_path = cli.config.unwrap_or_else(default_config_path);

    let config = load_config(&config_path)
        .with_context(|| format!("Failed to load relay configuration {:?}", config_path))?;

    let state = Arc::new(AppState::new(config));

    let app = Router::new()
        .route("/register", get(register_handler))
        .route("/connect", get(connect_handler))
        .route("/tunnel/:tunnel_id", get(tunnel_handler))
        .with_state(state.clone());

    let addr: SocketAddr = state.listen_addr.parse().context("Invalid bind address")?;

    info!("Starting relay on {}", state.listen_addr);

    axum::serve(
        tokio::net::TcpListener::bind(addr).await?,
        app.into_make_service(),
    )
    .await?;

    Ok(())
}

#[derive(Clone)]
struct AppState {
    listen_addr: String,
    relay_host: String,
    secrets: Arc<HashMap<Uuid, String>>,
    registered_servers: Arc<RwLock<HashMap<Uuid, Arc<RegisteredServer>>>>,
    tunnels: Arc<RwLock<HashMap<Uuid, Arc<TunnelHandle>>>>,
    handshake_timeout: Duration,
}

struct RegisteredServer {
    control_tx: mpsc::Sender<ServerCommand>,
    decoding_key: Arc<DecodingKey>,
    public_key_base64: String,
}

enum ServerCommand {
    OpenTunnel {
        tunnel_id: Uuid,
        client_id: Uuid,
        preferred_protocol: &'static str,
        expires_at: u64,
    },
}

impl ServerCommand {
    fn into_message(self) -> Message {
        match self {
            ServerCommand::OpenTunnel {
                tunnel_id,
                client_id,
                preferred_protocol,
                expires_at,
            } => {
                let payload = serde_json::json!({
                    "type": "open_tunnel",
                    "tunnel_id": tunnel_id.to_string(),
                    "client_id": client_id.to_string(),
                    "preferred_protocol": preferred_protocol,
                    "expires_at": expires_at,
                });
                Message::Text(payload.to_string())
            }
        }
    }
}

struct TunnelHandle {
    state: Mutex<TunnelState>,
    client_ws: Mutex<Option<WebSocket>>,
    server_ws: Mutex<Option<WebSocket>>,
}

impl TunnelHandle {
    fn new(now: Instant) -> Self {
        Self {
            state: Mutex::new(TunnelState::new(now)),
            client_ws: Mutex::new(None),
            server_ws: Mutex::new(None),
        }
    }
}

impl AppState {
    fn new(config: RelayConfig) -> Self {
        let RelayConfig {
            bind_address,
            port,
            handshake_timeout_seconds,
            registration_secrets,
        } = config;

        let listen_addr = format!("{}:{}", bind_address, port);
        let relay_host = bind_address;
        let handshake_timeout = Duration::from_secs(handshake_timeout_seconds.max(1));

        Self {
            listen_addr,
            relay_host,
            secrets: Arc::new(registration_secrets),
            registered_servers: Arc::new(RwLock::new(HashMap::new())),
            tunnels: Arc::new(RwLock::new(HashMap::new())),
            handshake_timeout,
        }
    }

    async fn validate_secret(&self, server_id: &Uuid, provided: &str) -> bool {
        match self.secrets.get(server_id) {
            Some(expected) => {
                if subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), provided.as_bytes()).into() {
                    true
                } else {
                    false
                }
            }
            None => false,
        }
    }

    async fn upsert_server(&self, server_id: Uuid, server: Arc<RegisteredServer>) {
        self.registered_servers.write().await.insert(server_id, server);
    }

    async fn remove_server(&self, server_id: &Uuid) {
        self.registered_servers.write().await.remove(server_id);
    }

    async fn server_entry(&self, server_id: &Uuid) -> Option<Arc<RegisteredServer>> {
        self.registered_servers.read().await.get(server_id).cloned()
    }

    async fn create_tunnel(&self, tunnel_id: Uuid, now: Instant) -> Arc<TunnelHandle> {
        let entry = Arc::new(TunnelHandle::new(now));
        self.tunnels.write().await.insert(tunnel_id, entry.clone());
        entry
    }

    async fn get_tunnel(&self, tunnel_id: &Uuid) -> Option<Arc<TunnelHandle>> {
        self.tunnels.read().await.get(tunnel_id).cloned()
    }

    async fn remove_tunnel(&self, tunnel_id: &Uuid) {
        self.tunnels.write().await.remove(tunnel_id);
    }
}

async fn register_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(err) = handle_register_socket(socket, state).await {
            warn!("Register socket ended with error: {err:?}");
        }
    })
}

async fn connect_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(err) = handle_connect_socket(socket, state).await {
            warn!("Connect socket ended with error: {err:?}");
        }
    })
}

async fn tunnel_handler(
    ws: WebSocketUpgrade,
    Path(tunnel_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        match Uuid::parse_str(&tunnel_id) {
            Ok(tunnel_uuid) => {
                if let Err(err) = handle_tunnel_socket(socket, state.clone(), tunnel_uuid).await {
                    warn!("Tunnel socket error: {err:?}");
                    state.remove_tunnel(&tunnel_uuid).await;
                }
            }
            Err(err) => warn!("Invalid tunnel id received: {err}"),
        }
    })
}

#[derive(Debug, Deserialize)]
struct RegisterPayload {
    #[serde(rename = "type")]
    r#type: String,
    server_id: String,
    relay_secret: String,
    server_version: String,
    capabilities: Vec<String>,
    public_key: String,
    max_tunnels: Option<u32>,
}

#[derive(Debug, Serialize)]
struct RegisterAck {
    #[serde(rename = "type")]
    r#type: &'static str,
    status: &'static str,
    retry_after_seconds: u32,
}

#[derive(Debug, Deserialize)]
struct ConnectPayload {
    #[serde(rename = "type")]
    r#type: String,
    server_id: String,
    relay_token: String,
    client_id: String,
    client_version: String,
}

#[derive(Debug, Serialize)]
struct ConnectAck {
    #[serde(rename = "type")]
    r#type: &'static str,
    status: &'static str,
    tunnel_id: String,
    relay_host: String,
    expires_at: u64,
}

#[derive(Debug, Deserialize)]
struct TunnelReadyPayload {
    #[serde(rename = "type")]
    r#type: String,
    tunnel_id: String,
    role: String,
}

#[derive(Clone, Debug, Deserialize)]
struct RelayClaims {
    iss: String,
    sub: String,
    aud: String,
    exp: u64,
    iat: u64,
    server_id: String,
    permissions: Vec<String>,
}

async fn handle_register_socket(mut socket: WebSocket, state: Arc<AppState>) -> Result<()> {
    let Some(Ok(Message::Text(payload))) = socket.recv().await else {
        anyhow::bail!("register socket closed before payload");
    };

    let msg: RegisterPayload =
        serde_json::from_str(&payload).context("Failed to parse register payload")?;

    if msg.r#type != "register" {
        anyhow::bail!("unexpected message type {}", msg.r#type);
    }

    let server_id = Uuid::parse_str(&msg.server_id).context("invalid server_id")?;

    if !state.validate_secret(&server_id, &msg.relay_secret).await {
        warn!("Server {} failed relay secret validation", server_id);
        let _ = socket
            .send(Message::Text(
                serde_json::json!({
                    "type": "register_ack",
                    "status": "error",
                    "error": "unauthorized"
                })
                .to_string(),
            ))
            .await;
        let _ = socket.close().await;
        return Ok(());
    }

    let raw_key = Base64
        .decode(msg.public_key.as_bytes())
        .context("public_key is not valid base64")?;
    let key_bytes: [u8; 32] = raw_key
        .try_into()
        .map_err(|_| anyhow!("public_key must be 32 bytes (Ed25519)"))?;
    let verifying_key = VerifyingKey::from_bytes(&key_bytes)
        .context("failed to construct verifying key")?;
    let public_key_der = verifying_key
        .to_public_key_der()
        .context("failed to convert verifying key to DER")?;
    let decoding_key = Arc::new(DecodingKey::from_ed_der(public_key_der.as_ref()));

    info!(
        "Registered server {} with capabilities {:?}",
        server_id, msg.capabilities
    );

    let ack = RegisterAck {
        r#type: "register_ack",
        status: "ok",
        retry_after_seconds: 0,
    };

    socket
        .send(Message::Text(serde_json::to_string(&ack)?))
        .await?;

    let (tx, mut rx) = mpsc::channel(32);
    let server_entry = Arc::new(RegisteredServer {
        control_tx: tx.clone(),
        decoding_key,
        public_key_base64: msg.public_key.clone(),
    });

    state.upsert_server(server_id, server_entry).await;

    loop {
        tokio::select! {
            maybe_cmd = rx.recv() => {
                match maybe_cmd {
                    Some(cmd) => {
                        if let Err(err) = socket.send(cmd.into_message()).await {
                            warn!("Control channel send failed for {server_id}: {err}");
                            break;
                        }
                    }
                    None => break,
                }
            }
            message = socket.recv() => {
                match message {
                    Some(Ok(Message::Ping(ping))) => {
                        let _ = socket.send(Message::Pong(ping)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        warn!("Control channel read failed for {server_id}: {err}");
                        break;
                    }
                }
            }
        }
    }

    state.remove_server(&server_id).await;
    Ok(())
}

async fn handle_connect_socket(mut socket: WebSocket, state: Arc<AppState>) -> Result<()> {
    let Some(Ok(Message::Text(payload))) = socket.recv().await else {
        anyhow::bail!("connect socket closed before payload");
    };

    let msg: ConnectPayload =
        serde_json::from_str(&payload).context("Failed to parse connect payload")?;

    if msg.r#type != "connect" {
        anyhow::bail!("unexpected message type {}", msg.r#type);
    }

    let server_id = Uuid::parse_str(&msg.server_id).context("invalid server_id")?;
    let client_id = Uuid::parse_str(&msg.client_id).context("invalid client_id")?;

    let Some(server_entry) = state.server_entry(&server_id).await else {
        warn!("Connect rejected for unregistered server {server_id}");
        let _ = socket
            .send(Message::Text(
                serde_json::json!({
                    "type": "connect_ack",
                    "status": "error",
                    "error": "server_not_registered"
                })
                .to_string(),
            ))
            .await;
        let _ = socket.close().await;
        return Ok(());
    };

    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_audience(&[state.relay_host.as_str()]);
    let claims = match decode::<RelayClaims>(
        &msg.relay_token,
        server_entry.decoding_key.as_ref(),
        &validation,
    ) {
        Ok(data) => data.claims,
        Err(err) => {
            warn!("Relay token validation failed: {err}");
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({
                        "type": "connect_ack",
                        "status": "error",
                        "error": "invalid_token"
                    })
                    .to_string(),
                ))
                .await;
            let _ = socket.close().await;
            return Ok(());
        }
    };

    let claims_server = match Uuid::parse_str(&claims.server_id) {
        Ok(uuid) => uuid,
        Err(_) => {
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({
                        "type": "connect_ack",
                        "status": "error",
                        "error": "invalid_token"
                    })
                    .to_string(),
                ))
                .await;
            let _ = socket.close().await;
            return Ok(());
        }
    };
    if claims_server != server_id {
        let _ = socket
            .send(Message::Text(
                serde_json::json!({
                    "type": "connect_ack",
                    "status": "error",
                    "error": "server_mismatch"
                })
                .to_string(),
            ))
            .await;
        let _ = socket.close().await;
        return Ok(());
    }

    if !claims.permissions.iter().any(|p| p == "connect") {
        let _ = socket
            .send(Message::Text(
                serde_json::json!({
                    "type": "connect_ack",
                    "status": "error",
                    "error": "permission_denied"
                })
                .to_string(),
            ))
            .await;
        let _ = socket.close().await;
        return Ok(());
    }

    let tunnel_id = Uuid::new_v4();
    let tunnel_entry = state.create_tunnel(tunnel_id, Instant::now()).await;

    let expires_at = SystemTime::now()
        .checked_add(state.handshake_timeout)
        .and_then(|deadline| deadline.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|dur| dur.as_secs())
        .unwrap_or(0);

    if server_entry
        .control_tx
        .send(ServerCommand::OpenTunnel {
            tunnel_id,
            client_id,
            preferred_protocol: "binary",
            expires_at,
        })
        .await
        .is_err()
    {
        warn!("Failed to notify server {server_id} about new tunnel");
        let _ = socket
            .send(Message::Text(
                serde_json::json!({
                    "type": "connect_ack",
                    "status": "error",
                    "error": "server_unreachable"
                })
                .to_string(),
            ))
            .await;
        let _ = socket.close().await;
        state.remove_tunnel(&tunnel_id).await;
        return Ok(());
    }

    let ack = ConnectAck {
        r#type: "connect_ack",
        status: "ok",
        tunnel_id: tunnel_id.to_string(),
        relay_host: state.relay_host.clone(),
        expires_at,
    };

    socket
        .send(Message::Text(serde_json::to_string(&ack)?))
        .await?;

    // Wait for tunnel_ready from client
    let Some(Ok(Message::Text(ready_payload))) = socket.recv().await else {
        anyhow::bail!("client closed before tunnel_ready");
    };

    let ready: TunnelReadyPayload =
        serde_json::from_str(&ready_payload).context("Failed to parse tunnel_ready")?;

    if ready.role != "client" || ready.tunnel_id != tunnel_id.to_string() {
        anyhow::bail!("invalid tunnel_ready payload from client");
    }

    {
        let mut guard = tunnel_entry.state.lock().await;
        guard.mark_client_ready(Instant::now());
    }

    info!(
        "Client {} waiting for server tunnel {}",
        client_id, tunnel_id
    );

    let deadline = Instant::now() + state.handshake_timeout;
    loop {
        {
            let guard = tunnel_entry.state.lock().await;
            if guard.is_fully_ready() {
                info!("Tunnel {} is now active", tunnel_id);
                break;
            }
            if guard.is_expired(Instant::now(), state.handshake_timeout) {
                warn!("Tunnel {} expired during handshake", tunnel_id);
                state.remove_tunnel(&tunnel_id).await;
                let _ = socket
                    .send(Message::Text(
                        serde_json::json!({
                            "type": "tunnel_failed",
                            "reason": "timeout"
                        })
                        .to_string(),
                    ))
                    .await;
                let _ = socket.close().await;
                return Ok(());
            }
        }

        if Instant::now() >= deadline {
            warn!("Tunnel {} handshake timed out", tunnel_id);
            state.remove_tunnel(&tunnel_id).await;
            let _ = socket.close().await;
            return Ok(());
        }

        sleep(Duration::from_millis(100)).await;
    }

    // Placeholder: real implementation would now proxy data frames.
    let _ = socket
        .send(Message::Text(
            serde_json::json!({
                "type": "tunnel_ready_ack",
                "tunnel_id": tunnel_id.to_string()
            })
            .to_string(),
        ))
        .await;

    state.remove_tunnel(&tunnel_id).await;

    Ok(())
}

async fn handle_tunnel_socket(
    mut socket: WebSocket,
    state: Arc<AppState>,
    tunnel_id: Uuid,
) -> Result<()> {
    let Some(entry) = state.get_tunnel(&tunnel_id).await else {
        warn!("Received tunnel for unknown id {}", tunnel_id);
        let _ = socket
            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                code: u16::from(CloseCode::Protocol),
                reason: "unknown_tunnel".into(),
            })))
            .await;
        return Ok(());
    };

    let Some(Ok(Message::Text(payload))) = socket.recv().await else {
        anyhow::bail!("server tunnel closed before tunnel_ready");
    };

    let ready: TunnelReadyPayload =
        serde_json::from_str(&payload).context("Failed to parse tunnel_ready from server")?;

    if ready.role != "server" || ready.tunnel_id != tunnel_id.to_string() {
        anyhow::bail!("invalid tunnel_ready payload from server");
    }

    {
        let mut guard = entry.state.lock().await;
        guard.mark_server_ready(Instant::now());
    }

    info!("Server confirmed tunnel {}", tunnel_id);

    // Wait until the client side notices activation or timeout.
    sleep(Duration::from_millis(200)).await;

    Ok(())
}
