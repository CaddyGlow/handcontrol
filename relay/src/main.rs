mod config;

use crate::config::{RelayConfig, default_config_path, load_config};
use anyhow::{Context, Result, anyhow};
use axum::{
    Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD as Base64};
use clap::{Parser, ValueHint};
use ed25519_dalek::{VerifyingKey, pkcs8::EncodePublicKey};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use handcontrol_relay::tunnel::state::TunnelState;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex, Notify, RwLock, mpsc};
use tokio::time::timeout;
use tracing::{info, warn};
use tungstenite::protocol::frame::coding::CloseCode;
use uuid::Uuid;

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

    let app = build_router(state.clone());

    let addr: SocketAddr = state.listen_addr.parse().context("Invalid bind address")?;

    info!("Starting relay on {}", state.listen_addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/register", get(register_handler))
        .route("/connect", get(connect_handler))
        .route("/tunnel/:tunnel_id", get(tunnel_handler))
        .with_state(state)
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
    notify: Notify,
}

impl TunnelHandle {
    fn new(now: Instant) -> Self {
        Self {
            state: Mutex::new(TunnelState::new(now)),
            client_ws: Mutex::new(None),
            server_ws: Mutex::new(None),
            notify: Notify::new(),
        }
    }

    async fn attach_client(&self, ws: WebSocket) -> Option<(WebSocket, WebSocket)> {
        {
            let mut guard = self.client_ws.lock().await;
            *guard = Some(ws);
        }
        self.notify.notify_waiters();
        self.take_pair_if_ready().await
    }

    async fn attach_server(&self, ws: WebSocket) -> Option<(WebSocket, WebSocket)> {
        {
            let mut guard = self.server_ws.lock().await;
            *guard = Some(ws);
        }
        self.notify.notify_waiters();
        self.take_pair_if_ready().await
    }

    async fn take_pair_if_ready(&self) -> Option<(WebSocket, WebSocket)> {
        let client_opt = {
            let mut guard = self.client_ws.lock().await;
            guard.take()
        };

        let Some(client_ws) = client_opt else {
            return None;
        };

        let server_opt = {
            let mut guard = self.server_ws.lock().await;
            guard.take()
        };

        match server_opt {
            Some(server_ws) => Some((client_ws, server_ws)),
            None => {
                let mut guard = self.client_ws.lock().await;
                *guard = Some(client_ws);
                None
            }
        }
    }

    async fn take_client_socket(&self) -> Option<WebSocket> {
        self.client_ws.lock().await.take()
    }

    async fn take_server_socket(&self) -> Option<WebSocket> {
        self.server_ws.lock().await.take()
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
                if ConstantTimeEq::ct_eq(expected.as_bytes(), provided.as_bytes()).into() {
                    true
                } else {
                    false
                }
            }
            None => false,
        }
    }

    async fn upsert_server(&self, server_id: Uuid, server: Arc<RegisteredServer>) {
        self.registered_servers
            .write()
            .await
            .insert(server_id, server);
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct TunnelReadyPayload {
    #[serde(rename = "type")]
    r#type: String,
    tunnel_id: String,
    role: String,
}

#[allow(dead_code)]
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
    let verifying_key =
        VerifyingKey::from_bytes(&key_bytes).context("failed to construct verifying key")?;
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
    tunnel_entry.notify.notify_waiters();

    info!(
        "Client {} waiting for server tunnel {}",
        client_id, tunnel_id
    );

    if let Some((client_ws, server_ws)) = tunnel_entry.attach_client(socket).await {
        info!("Tunnel {} became active immediately", tunnel_id);
        spawn_forwarders(state.clone(), tunnel_id, client_ws, server_ws);
        return Ok(());
    }

    let handle_wait = tunnel_entry.clone();
    let handle_cleanup = tunnel_entry.clone();
    let state_clone = state.clone();
    let wait_result = timeout(state.handshake_timeout, async move {
        loop {
            handle_wait.notify.notified().await;
            if let Some((client_ws, server_ws)) = handle_wait.take_pair_if_ready().await {
                info!("Tunnel {} is now active", tunnel_id);
                spawn_forwarders(state_clone.clone(), tunnel_id, client_ws, server_ws);
                break;
            }
        }
    })
    .await;

    if wait_result.is_err() {
        warn!("Tunnel {} expired waiting for server", tunnel_id);
        if let Some(mut client_ws) = handle_cleanup.take_client_socket().await {
            let _ = client_ws
                .send(Message::Text(
                    serde_json::json!({
                        "type": "tunnel_failed",
                        "reason": "timeout"
                    })
                    .to_string(),
                ))
                .await;
            let _ = client_ws.close().await;
        }
        state.remove_tunnel(&tunnel_id).await;
    }

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
    entry.notify.notify_waiters();

    info!("Server confirmed tunnel {}", tunnel_id);

    if let Some((client_ws, server_ws)) = entry.attach_server(socket).await {
        spawn_forwarders(state.clone(), tunnel_id, client_ws, server_ws);
        return Ok(());
    }

    let handle_wait = entry.clone();
    let handle_cleanup = entry.clone();
    let state_clone = state.clone();
    let wait_result = timeout(state.handshake_timeout, async move {
        loop {
            handle_wait.notify.notified().await;
            if let Some((client_ws, server_ws)) = handle_wait.take_pair_if_ready().await {
                spawn_forwarders(state_clone.clone(), tunnel_id, client_ws, server_ws);
                break;
            }
        }
    })
    .await;

    if wait_result.is_err() {
        warn!("Tunnel {} expired waiting for client", tunnel_id);
        if let Some(mut server_ws) = handle_cleanup.take_server_socket().await {
            let _ = server_ws
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: u16::from(CloseCode::Normal),
                    reason: "timeout".into(),
                })))
                .await;
        }
        state.remove_tunnel(&tunnel_id).await;
    }

    Ok(())
}

fn spawn_forwarders(
    state: Arc<AppState>,
    tunnel_id: Uuid,
    client_ws: WebSocket,
    server_ws: WebSocket,
) {
    tokio::spawn(async move {
        if let Err(err) = forward_bidirectional(client_ws, server_ws).await {
            warn!("Tunnel {tunnel_id} forwarding error: {err:?}");
        }
        state.remove_tunnel(&tunnel_id).await;
    });
}

async fn forward_bidirectional(client_ws: WebSocket, server_ws: WebSocket) -> Result<()> {
    let (client_sink, client_stream) = client_ws.split();
    let (server_sink, server_stream) = server_ws.split();

    let client_to_server =
        tokio::spawn(async move { forward_stream(client_stream, server_sink).await });
    let server_to_client =
        tokio::spawn(async move { forward_stream(server_stream, client_sink).await });

    let (c_res, s_res) = tokio::join!(client_to_server, server_to_client);
    c_res??;
    s_res??;
    Ok(())
}

async fn forward_stream(
    mut inbound: SplitStream<WebSocket>,
    mut outbound: SplitSink<WebSocket, Message>,
) -> Result<()> {
    while let Some(msg) = inbound.next().await {
        match msg {
            Ok(Message::Binary(bytes)) => {
                outbound
                    .send(Message::Binary(bytes))
                    .await
                    .map_err(|err| anyhow!(err))?;
            }
            Ok(Message::Close(frame)) => {
                let _ = outbound.send(Message::Close(frame.clone())).await;
                break;
            }
            Ok(Message::Ping(data)) => {
                outbound
                    .send(Message::Ping(data))
                    .await
                    .map_err(|err| anyhow!(err))?;
            }
            Ok(Message::Pong(data)) => {
                outbound
                    .send(Message::Pong(data))
                    .await
                    .map_err(|err| anyhow!(err))?;
            }
            Ok(Message::Text(_)) => {
                // Ignore text frames; relay tunnels only forward binary gRPC frames.
            }
            Err(err) => return Err(anyhow!(err)),
        }
    }

    let _ = outbound.send(Message::Close(None)).await;
    Ok(())
}

#[cfg(all(test, feature = "relay-integration"))]
mod tests {
    use super::*;
    use crate::config::RelayConfig;
    use ed25519_dalek::{SigningKey, pkcs8::EncodePrivateKey};
    use jsonwebtoken::{Algorithm, EncodingKey, Header};
    use serde::Serialize;
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use tokio::{net::TcpListener, sync::oneshot, time::Duration};
    use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

    #[derive(Serialize)]
    struct TestClaims {
        iss: &'static str,
        sub: &'static str,
        aud: String,
        exp: u64,
        iat: u64,
        server_id: String,
        permissions: Vec<&'static str>,
    }

    fn test_config(server_id: Uuid, secret: &str, timeout_secs: u64) -> RelayConfig {
        let mut secrets = HashMap::new();
        secrets.insert(server_id, secret.to_string());
        RelayConfig {
            bind_address: "127.0.0.1".to_string(),
            port: 0,
            handshake_timeout_seconds: timeout_secs,
            registration_secrets: secrets,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn tunnel_roundtrip_binary_data() {
        let server_id = Uuid::new_v4();
        let secret = Base64.encode(b"relayed-secret".as_ref());
        let config = test_config(server_id, &secret, 5);
        let state = Arc::new(AppState::new(config));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app_state = state.clone();
        let server_handle = tokio::spawn(async move {
            axum::serve(listener, build_router(app_state).into_make_service())
                .await
                .unwrap();
        });

        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let public_key_base64 = Base64.encode(verifying_key.to_bytes());
        let private_der = signing_key.to_pkcs8_der().unwrap();
        let encoding_key = EncodingKey::from_ed_der(private_der.as_bytes());

        // Register server control channel
        let register_url = format!("ws://{addr}/register");
        let (control_ws, _) = connect_async(register_url).await.unwrap();
        let (mut control_sink, mut control_stream) = control_ws.split();

        let register_msg = json!({
            "type": "register",
            "server_id": server_id.to_string(),
            "relay_secret": secret,
            "server_version": "test",
            "capabilities": ["relay.v1"],
            "public_key": public_key_base64,
            "max_tunnels": 4,
        });
        control_sink
            .send(WsMessage::Text(register_msg.to_string().into()))
            .await
            .unwrap();

        let ack_msg = control_stream.next().await.unwrap().unwrap();
        let ack_text = ack_msg.into_text().unwrap().to_string();
        let ack_value: Value = serde_json::from_str(&ack_text).unwrap();
        assert_eq!(
            ack_value["status"], "ok",
            "connect ack error: {ack_value:?}"
        );

        let (open_tunnel_tx, open_tunnel_rx) = oneshot::channel();
        let control_task = tokio::spawn(async move {
            let mut open_tunnel_tx = Some(open_tunnel_tx);
            let mut control_sink = control_sink;
            let mut control_stream = control_stream;
            while let Some(msg) = control_stream.next().await {
                match msg {
                    Ok(WsMessage::Text(text)) => {
                        let owned = text.to_string();
                        if let Ok(value) = serde_json::from_str::<Value>(&owned) {
                            if value["type"] == "open_tunnel" {
                                if let (Some(tx), Some(tunnel_id_str)) =
                                    (open_tunnel_tx.take(), value["tunnel_id"].as_str())
                                {
                                    let tunnel_id = Uuid::parse_str(tunnel_id_str).unwrap();
                                    let _ = tx.send(tunnel_id);
                                }
                            }
                        }
                    }
                    Ok(WsMessage::Ping(data)) => {
                        let _ = control_sink.send(WsMessage::Pong(data)).await;
                    }
                    Ok(WsMessage::Close(_)) | Err(_) => break,
                    _ => {}
                }
            }
        });

        // Client connects (no server tunnel yet)
        let connect_url = format!("ws://{addr}/connect");
        let (mut client_ws, _) = connect_async(connect_url).await.unwrap();

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = TestClaims {
            iss: "handcontrol-server",
            sub: "test-client",
            aud: "127.0.0.1".to_string(),
            exp: now + 60,
            iat: now,
            server_id: server_id.to_string(),
            permissions: vec!["connect"],
        };
        let token =
            jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), &claims, &encoding_key).unwrap();

        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_audience(&["127.0.0.1"]);
        decode::<RelayClaims>(
            &token,
            &DecodingKey::from_ed_der(verifying_key.to_public_key_der().unwrap().as_bytes()),
            &validation,
        )
        .expect("token should decode locally");

        let client_id = Uuid::new_v4();
        let connect_msg = json!({
            "type": "connect",
            "server_id": server_id.to_string(),
            "relay_token": token,
            "client_id": client_id.to_string(),
            "client_version": "integration-test",
        });
        client_ws
            .send(WsMessage::Text(connect_msg.to_string().into()))
            .await
            .unwrap();

        let ack_msg = client_ws.next().await.unwrap().unwrap();
        let ack_text = ack_msg.into_text().unwrap().to_string();
        let ack_value: Value = serde_json::from_str(&ack_text).unwrap();
        assert_eq!(
            ack_value["status"], "ok",
            "connect ack error: {ack_value:?}"
        );
        let tunnel_id = Uuid::parse_str(ack_value["tunnel_id"].as_str().unwrap()).unwrap();

        let ready_msg = json!({
            "type": "tunnel_ready",
            "tunnel_id": tunnel_id.to_string(),
            "role": "client",
        });
        client_ws
            .send(WsMessage::Text(ready_msg.to_string().into()))
            .await
            .unwrap();

        let tunnel_id_notify = open_tunnel_rx.await.unwrap();
        assert_eq!(tunnel_id, tunnel_id_notify);

        let server_tunnel_url = format!("ws://{addr}/tunnel/{tunnel_id}?role=server");
        let (server_tunnel_ws, _) = connect_async(server_tunnel_url).await.unwrap();
        let (mut server_sink, mut server_stream) = server_tunnel_ws.split();
        let ready_msg = json!({
            "type": "tunnel_ready",
            "tunnel_id": tunnel_id.to_string(),
            "role": "server",
        });
        server_sink
            .send(WsMessage::Text(ready_msg.to_string().into()))
            .await
            .unwrap();

        let server_forward_task = tokio::spawn(async move {
            if let Some(Ok(WsMessage::Binary(data))) = server_stream.next().await {
                let bytes = data.as_slice().to_vec();
                assert_eq!(bytes, b"hello".to_vec());
                let _ = server_sink
                    .send(WsMessage::Binary(b"world".to_vec().into()))
                    .await;
            }
        });

        client_ws
            .send(WsMessage::Binary(b"hello".to_vec().into()))
            .await
            .unwrap();

        let response = tokio::time::timeout(Duration::from_secs(2), client_ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match response {
            WsMessage::Binary(data) => {
                assert_eq!(data.as_slice(), b"world");
            }
            other => panic!("expected binary message, got {other:?}"),
        }

        let _ = client_ws.close(None).await;
        let _ = control_task.await;
        let _ = server_forward_task.await;
        server_handle.abort();
        let _ = server_handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn tunnel_times_out_without_server() {
        let server_id = Uuid::new_v4();
        let secret = Base64.encode(b"relayed-secret".as_ref());
        let config = test_config(server_id, &secret, 1);
        let state = Arc::new(AppState::new(config));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app_state = state.clone();
        let server_handle = tokio::spawn(async move {
            axum::serve(listener, build_router(app_state).into_make_service())
                .await
                .unwrap();
        });

        let signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let public_key_base64 = Base64.encode(verifying_key.to_bytes());
        let private_der = signing_key.to_pkcs8_der().unwrap();
        let encoding_key = EncodingKey::from_ed_der(private_der.as_bytes());

        // Register server
        let register_url = format!("ws://{addr}/register");
        let (control_ws, _) = connect_async(register_url).await.unwrap();
        let (mut control_sink, mut control_stream) = control_ws.split();

        let register_msg = json!({
            "type": "register",
            "server_id": server_id.to_string(),
            "relay_secret": secret,
            "server_version": "test",
            "capabilities": ["relay.v1"],
            "public_key": public_key_base64,
            "max_tunnels": 1,
        });
        control_sink
            .send(WsMessage::Text(register_msg.to_string().into()))
            .await
            .unwrap();

        let ack_msg = control_stream.next().await.unwrap().unwrap();
        let ack_text = ack_msg.into_text().unwrap().to_string();
        let ack_value: Value = serde_json::from_str(&ack_text).unwrap();
        assert_eq!(ack_value["status"], "ok");

        let control_task = tokio::spawn(async move {
            while let Some(msg) = control_stream.next().await {
                match msg {
                    Ok(WsMessage::Ping(data)) => {
                        let _ = control_sink.send(WsMessage::Pong(data)).await;
                    }
                    Ok(WsMessage::Close(_)) | Err(_) => break,
                    _ => {}
                }
            }
        });

        // Client connects but server never opens tunnel
        let connect_url = format!("ws://{addr}/connect");
        let (mut client_ws, _) = connect_async(connect_url).await.unwrap();

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = TestClaims {
            iss: "handcontrol-server",
            sub: "test-client",
            aud: "127.0.0.1".to_string(),
            exp: now + 2,
            iat: now,
            server_id: server_id.to_string(),
            permissions: vec!["connect"],
        };
        let token =
            jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), &claims, &encoding_key).unwrap();

        let connect_msg = json!({
            "type": "connect",
            "server_id": server_id.to_string(),
            "relay_token": token,
            "client_id": Uuid::new_v4().to_string(),
            "client_version": "integration-test",
        });
        client_ws
            .send(WsMessage::Text(connect_msg.to_string().into()))
            .await
            .unwrap();

        let ack_msg = client_ws.next().await.unwrap().unwrap();
        let ack_text = ack_msg.into_text().unwrap().to_string();
        let ack_value: Value = serde_json::from_str(&ack_text).unwrap();
        assert_eq!(ack_value["status"], "ok");
        let tunnel_id = ack_value["tunnel_id"].as_str().unwrap();

        let ready_msg = json!({
            "type": "tunnel_ready",
            "tunnel_id": tunnel_id,
            "role": "client",
        });
        client_ws
            .send(WsMessage::Text(ready_msg.to_string().into()))
            .await
            .unwrap();

        let failure = tokio::time::timeout(Duration::from_secs(3), client_ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let failure_text = failure.into_text().unwrap().to_string();
        let failure_value: Value = serde_json::from_str(&failure_text).unwrap();
        assert_eq!(failure_value["type"], "tunnel_failed");
        assert_eq!(failure_value["reason"], "timeout");

        let _ = client_ws.close(None).await;
        let _ = control_task.await;
        server_handle.abort();
        let _ = server_handle.await;
    }
}
