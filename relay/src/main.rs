mod config;

use crate::config::{RelayConfig, default_config_path, load_config};
use anyhow::{Context, Result, anyhow, bail};
use axum::{
    Router,
    body::Body,
    extract::{
        Path, Query, State,
        connect_info::ConnectInfo,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{Request, Version, header},
    middleware,
    middleware::Next,
    response::IntoResponse,
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD as Base64};
use clap::{Parser, ValueHint};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use handcontrol_relay::tunnel::state::TunnelState;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use quinn::{Endpoint, ReadExactError};
use rand::RngCore;
use rustls::crypto;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::File,
    io::BufReader,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex, Notify, RwLock, mpsc};
use tokio::time::timeout;
use tracing::{Span, field, instrument};
use tracing::{debug, info, trace, warn};
use tungstenite::protocol::frame::coding::CloseCode;
use url::Url;
use uuid::Uuid;

const RELAY_SUBPROTOCOL: &str = "handcontrol-relay.v1";

#[derive(Parser)]
#[command(name = "handcontrol-relay")]
struct Cli {
    /// Path to relay configuration file
    #[arg(long, value_hint = ValueHint::FilePath)]
    config: Option<std::path::PathBuf>,
    /// Allow any relay token audience (for development/testing)
    #[arg(long, default_value_t = false)]
    allow_all_audiences: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = crypto::ring::default_provider().install_default();
    let cli = Cli::parse();
    tracing_subscriber::fmt::init();

    let config_path = cli.config.unwrap_or_else(default_config_path);

    let mut config = load_config(&config_path)
        .with_context(|| format!("Failed to load relay configuration {:?}", config_path))?;

    if cli.allow_all_audiences {
        config.allow_all_audiences = true;
        tracing::warn!("allow-all-audiences flag enabled; audience validation will be skipped");
    }

    // Keep TLS paths before moving config
    let tls_cert_path = config.tls_cert_path.clone();
    let tls_key_path = config.tls_key_path.clone();

    let state = Arc::new(AppState::new(config));

    if let Some(port) = state.quic_port {
        match (tls_cert_path.as_ref(), tls_key_path.as_ref()) {
            (Some(cert), Some(key)) => {
                if let Err(err) =
                    start_quic_listener(state.clone(), port, cert.as_path(), key.as_path())
                {
                    warn!("Failed to start QUIC listener: {err:#}");
                }
            }
            _ => {
                warn!(
                    "QUIC port configured but TLS certificate/key not provided; skipping QUIC listener"
                );
            }
        }
    }

    let app = build_router(state.clone());

    let addr: SocketAddr = state.listen_addr.parse().context("Invalid bind address")?;

    // Check if TLS is configured
    match (tls_cert_path, tls_key_path) {
        (Some(cert_path), Some(key_path)) => {
            info!("Starting relay with TLS on {}", state.listen_addr);
            serve_with_tls(addr, app, cert_path, key_path).await?;
        }
        (None, None) => {
            info!("Starting relay without TLS on {}", state.listen_addr);
            let listener = tokio::net::TcpListener::bind(addr).await?;
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await?;
        }
        _ => {
            anyhow::bail!(
                "Both tls_cert_path and tls_key_path must be specified for TLS, or neither for plain HTTP"
            );
        }
    }

    Ok(())
}

fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/register", get(register_handler))
        .route("/connect", get(connect_handler))
        .route("/tunnel/:tunnel_id", get(tunnel_handler))
        .layer(middleware::from_fn(trace_http_requests))
        .with_state(state)
}

async fn trace_http_requests(req: Request<Body>, next: Next) -> impl IntoResponse {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let version = req.version();
    let headers = req.headers();

    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_owned())
        .unwrap_or_else(|| "-".to_string());
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_owned())
        .unwrap_or_else(|| "-".to_string());
    let scheme = uri
        .scheme_str()
        .map(|value| value.to_owned())
        .or_else(|| {
            headers
                .get("x-forwarded-proto")
                .and_then(|value| value.to_str().ok().map(|value| value.to_owned()))
        })
        .unwrap_or_else(|| "-".to_string());

    let remote_addr = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.to_string())
        .unwrap_or_else(|| "-".to_string());

    let path = uri.path().to_string();
    let query = uri.query().unwrap_or("");
    let protocol = http_version_label(version);

    trace!(
        target: "handcontrol_relay::http",
        remote_addr = %remote_addr,
        host = %host,
        user_agent = %user_agent,
        method = %method,
        path = %path,
        query = query,
        scheme = %scheme,
        protocol = protocol,
        "Incoming HTTP request"
    );

    next.run(req).await
}

fn http_version_label(version: Version) -> &'static str {
    match version {
        Version::HTTP_09 => "HTTP/0.9",
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_11 => "HTTP/1.1",
        Version::HTTP_2 => "HTTP/2",
        Version::HTTP_3 => "HTTP/3",
        _ => "HTTP/UNKNOWN",
    }
}

async fn serve_with_tls(
    addr: SocketAddr,
    app: Router,
    cert_path: std::path::PathBuf,
    key_path: std::path::PathBuf,
) -> Result<()> {
    use rustls_pemfile::{certs, private_key};
    use std::fs::File;
    use std::io::BufReader;

    // Load TLS certificate
    let cert_file = File::open(&cert_path)
        .with_context(|| format!("Failed to open certificate file: {}", cert_path.display()))?;
    let mut cert_reader = BufReader::new(cert_file);
    let certs: Vec<_> = certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse certificate")?;

    if certs.is_empty() {
        anyhow::bail!("No certificates found in {}", cert_path.display());
    }

    // Load TLS private key
    let key_file = File::open(&key_path)
        .with_context(|| format!("Failed to open private key file: {}", key_path.display()))?;
    let mut key_reader = BufReader::new(key_file);
    let key = private_key(&mut key_reader)
        .context("Failed to read private key")?
        .context("No private key found")?;

    // Configure TLS
    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("Failed to build TLS config")?;
    server_config.alpn_protocols = vec![
        b"h2".to_vec(),
        b"http/1.1".to_vec(),
        RELAY_SUBPROTOCOL.as_bytes().to_vec(),
    ];

    let tls_config = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config));

    // Serve with TLS using axum-server
    axum_server::bind_rustls(addr, tls_config)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context("TLS server failed")
}

fn start_quic_listener(
    state: Arc<AppState>,
    quic_port: u16,
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
) -> Result<()> {
    let server_config = build_quic_server_config(cert_path, key_path)?;
    let bind_ip = state
        .bind_address
        .parse::<IpAddr>()
        .unwrap_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED));
    let quic_addr = SocketAddr::new(bind_ip, quic_port);

    info!("Starting QUIC listener on {}", quic_addr);
    let endpoint = Endpoint::server(server_config, quic_addr)
        .with_context(|| format!("Failed to bind QUIC listener on {}", quic_addr))?;

    let accept_state = Arc::clone(&state);
    tokio::spawn(async move {
        if let Err(err) = run_quic_accept_loop(endpoint, accept_state).await {
            warn!("QUIC listener stopped: {err:#}");
        }
    });

    Ok(())
}

fn build_quic_server_config(
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
) -> Result<quinn::ServerConfig> {
    use rustls_pemfile::{certs, private_key};

    let cert_file = File::open(cert_path)
        .with_context(|| format!("Failed to open QUIC certificate {}", cert_path.display()))?;
    let mut cert_reader = BufReader::new(cert_file);
    let cert_chain: Vec<CertificateDer<'static>> = certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse QUIC certificate chain")?;
    if cert_chain.is_empty() {
        bail!(
            "No certificates found in QUIC certificate file {}",
            cert_path.display()
        );
    }

    let key_file = File::open(key_path)
        .with_context(|| format!("Failed to open QUIC private key {}", key_path.display()))?;
    let mut key_reader = BufReader::new(key_file);
    let key: PrivateKeyDer<'static> = private_key(&mut key_reader)
        .context("Failed to read QUIC private key")?
        .context("No private key found for QUIC listener")?;

    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .context("Failed to build QUIC TLS config")?;
    server_crypto
        .alpn_protocols
        .push(RELAY_SUBPROTOCOL.as_bytes().to_vec());

    let quic_crypto = quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
        .context("Failed to adapt TLS config for QUIC")?;
    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));
    server_config.transport_config(Arc::new(quinn::TransportConfig::default()));
    Ok(server_config)
}

async fn run_quic_accept_loop(endpoint: Endpoint, state: Arc<AppState>) -> Result<()> {
    while let Some(connecting) = endpoint.accept().await {
        let state_clone = Arc::clone(&state);
        tokio::spawn(async move {
            match connecting.await {
                Ok(connection) => {
                    if let Err(err) = process_quic_connection(connection, state_clone).await {
                        warn!("QUIC connection terminated with error: {err:#}");
                    }
                }
                Err(err) => {
                    warn!("QUIC handshake failed: {err}");
                }
            }
        });
    }

    Ok(())
}

async fn process_quic_connection(
    connection: quinn::Connection,
    state: Arc<AppState>,
) -> Result<()> {
    let remote = connection.remote_address();
    trace!(%remote, "Accepted QUIC connection");

    let (send, mut recv) = connection
        .accept_bi()
        .await
        .context("Failed to accept QUIC bidirectional stream")?;
    let first_frame = quic_read_frame(&mut recv).await?;
    let Some(QuicFrame::Text(payload)) = first_frame else {
        bail!(
            "QUIC connection {} did not start with a control frame",
            remote
        );
    };

    let message_type = serde_json::from_str::<serde_json::Value>(&payload)
        .context("Failed to parse QUIC control JSON")?
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();

    match message_type.as_str() {
        "register" => {
            let register: RegisterPayload =
                serde_json::from_str(&payload).context("Failed to deserialize register payload")?;
            handle_quic_register(state, register, send, recv).await
        }
        "connect" => {
            let connect: ConnectPayload =
                serde_json::from_str(&payload).context("Failed to deserialize connect payload")?;
            handle_quic_connect(state, connect, send, recv).await
        }
        "tunnel_ready" => {
            let ready: TunnelReadyPayload = serde_json::from_str(&payload)
                .context("Failed to deserialize tunnel_ready payload")?;
            handle_quic_server_tunnel(state, ready, send, recv).await
        }
        other => bail!("Unexpected QUIC control message type '{other}'"),
    }
}

async fn handle_quic_register(
    state: Arc<AppState>,
    payload: RegisterPayload,
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
) -> Result<()> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    if payload.r#type != "register" {
        bail!("Unexpected message type {}", payload.r#type);
    }

    let server_id = Uuid::parse_str(&payload.server_id).context("invalid server_id")?;
    if !state
        .validate_secret(&server_id, &payload.relay_secret)
        .await
    {
        warn!("Server {} failed relay secret validation", server_id);
        let error_ack = serde_json::json!({
            "type": "register_ack",
            "status": "error",
            "error": "unauthorized"
        })
        .to_string();
        quic_write_frame(&mut send, QuicFrameType::Text, error_ack.as_bytes()).await?;
        let _ = send.finish();
        return Ok(());
    }

    let raw_key = Base64
        .decode(payload.public_key.as_bytes())
        .context("public_key is not valid base64")?;
    let key_bytes: [u8; 32] = raw_key
        .try_into()
        .map_err(|_| anyhow!("public_key must be 32 bytes (Ed25519)"))?;

    let public_key_b64url = URL_SAFE_NO_PAD.encode(&key_bytes);
    let decoding_key = Arc::new(
        DecodingKey::from_ed_components(&public_key_b64url)
            .context("failed to create decoding key from public key")?,
    );

    info!(
        "Registered server {} with capabilities {:?}",
        server_id, payload.capabilities
    );

    let tls_authority = payload
        .tls_authority
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    let ack = RegisterAck {
        r#type: "register_ack",
        status: "ok",
        retry_after_seconds: 0,
    };
    let ack_json = serde_json::to_string(&ack)?;
    quic_write_frame(&mut send, QuicFrameType::Text, ack_json.as_bytes()).await?;

    let send = Arc::new(Mutex::new(send));
    let (tx, mut rx) = mpsc::channel(32);
    let registration_id = Uuid::new_v4();
    let server_entry = Arc::new(RegisteredServer {
        registration_id,
        control_tx: tx.clone(),
        decoding_key,
        tls_authority,
    });

    state.upsert_server(server_id, server_entry.clone()).await;

    loop {
        tokio::select! {
            maybe_cmd = rx.recv() => {
                match maybe_cmd {
                    Some(ServerCommand::OpenTunnel { tunnel_id, client_id, preferred_protocol, expires_at, server_secret }) => {
                        trace!("Dispatching control command to server {}", server_id);
                        let payload = serde_json::json!({
                            "type": "open_tunnel",
                            "tunnel_id": tunnel_id.to_string(),
                            "client_id": client_id.to_string(),
                            "preferred_protocol": preferred_protocol,
                            "expires_at": expires_at,
                            "server_secret": server_secret,
                        })
                        .to_string();
                        let mut guard = send.lock().await;
                        if let Err(err) =
                            quic_write_frame(&mut *guard, QuicFrameType::Text, payload.as_bytes())
                                .await
                        {
                            warn!("Control channel send failed for {server_id}: {err}");
                            break;
                        }
                    }
                    None => break,
                }
            }
            frame = quic_read_frame(&mut recv) => {
                match frame {
                    Ok(Some(QuicFrame::Ping(payload))) => {
                        let mut guard = send.lock().await;
                        let _ = quic_write_frame(&mut *guard, QuicFrameType::Pong, payload.as_slice()).await;
                        trace!("Control channel ping handled for server {}", server_id);
                    }
                Ok(Some(QuicFrame::Pong(payload))) => {
                    trace!(
                        "Control channel pong received for server {} ({} bytes)",
                        server_id,
                        payload.len()
                    );
                }
                    Ok(Some(QuicFrame::Close)) | Ok(None) => break,
                    Ok(Some(QuicFrame::Binary(_))) => {
                        trace!("Ignoring unexpected binary frame on register control channel");
                    }
                    Ok(Some(QuicFrame::Text(_))) => {
                        trace!("Ignoring unexpected text frame on register control channel");
                    }
                    Err(err) => {
                        warn!("Control channel read failed for {server_id}: {err}");
                        break;
                    }
                }
            }
        }
    }

    state
        .remove_server_if_current(&server_id, &registration_id)
        .await;
    trace!("Removed server registration {}", server_id);
    Ok(())
}

async fn handle_quic_connect(
    state: Arc<AppState>,
    payload: ConnectPayload,
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
) -> Result<()> {
    let server_id = Uuid::parse_str(&payload.server_id).context("invalid server_id")?;
    let client_id = Uuid::parse_str(&payload.client_id).context("invalid client_id")?;
    let client_id_str = client_id.to_string();
    trace!(
        "QUIC connect request server={} client={}",
        server_id, client_id
    );

    let Some(server_entry) = state.server_entry(&server_id).await else {
        warn!("Connect rejected for unregistered server {server_id}");
        let error_ack = serde_json::json!({
            "type": "connect_ack",
            "status": "error",
            "error": "server_not_registered"
        })
        .to_string();
        quic_write_frame(&mut send, QuicFrameType::Text, error_ack.as_bytes()).await?;
        let _ = send.finish();
        return Ok(());
    };

    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.validate_aud = false;
    let claims = match decode::<RelayClaims>(
        &payload.relay_token,
        server_entry.decoding_key.as_ref(),
        &validation,
    ) {
        Ok(data) => data.claims,
        Err(err) => {
            warn!("Relay token validation failed: {err}");
            let error_ack = serde_json::json!({
                "type": "connect_ack",
                "status": "error",
                "error": "invalid_token"
            })
            .to_string();
            quic_write_frame(&mut send, QuicFrameType::Text, error_ack.as_bytes()).await?;
            let _ = send.finish();
            return Ok(());
        }
    };

    trace!("Relay token validated for client {}", client_id);

    if !state.is_allowed_audience(&claims.aud) {
        warn!(
            "Relay token audience '{}' did not match expected host '{}'",
            claims.aud, state.relay_host
        );
        let error_ack = serde_json::json!({
            "type": "connect_ack",
            "status": "error",
            "error": "invalid_token"
        })
        .to_string();
        quic_write_frame(&mut send, QuicFrameType::Text, error_ack.as_bytes()).await?;
        let _ = send.finish();
        return Ok(());
    }

    let binding_type = claims.binding_type.as_deref().unwrap_or("client_id");
    let binding_value = claims.binding_value.as_deref().unwrap_or(&claims.sub);

    if claims.sub != binding_value || claims.sub != client_id_str {
        warn!(
            "Relay token subject mismatch (sub={}, binding={}, client={})",
            claims.sub, binding_value, client_id
        );
        let error_ack = serde_json::json!({
            "type": "connect_ack",
            "status": "error",
            "error": "binding_mismatch"
        })
        .to_string();
        quic_write_frame(&mut send, QuicFrameType::Text, error_ack.as_bytes()).await?;
        let _ = send.finish();
        return Ok(());
    }

    match binding_type {
        "client_id" | "enrollment_token" => {
            if binding_value != client_id_str {
                warn!(
                    "Relay token binding value '{}' did not match client {}",
                    binding_value, client_id
                );
                let error_ack = serde_json::json!({
                    "type": "connect_ack",
                    "status": "error",
                    "error": "binding_mismatch"
                })
                .to_string();
                quic_write_frame(&mut send, QuicFrameType::Text, error_ack.as_bytes()).await?;
                let _ = send.finish();
                return Ok(());
            }
        }
        other => {
            warn!("Unsupported relay token binding type '{}'", other);
            let error_ack = serde_json::json!({
                "type": "connect_ack",
                "status": "error",
                "error": "invalid_token"
            })
            .to_string();
            quic_write_frame(&mut send, QuicFrameType::Text, error_ack.as_bytes()).await?;
            let _ = send.finish();
            return Ok(());
        }
    }

    let claims_server = Uuid::parse_str(&claims.server_id).context("invalid server_id in token")?;
    if claims_server != server_id {
        let error_ack = serde_json::json!({
            "type": "connect_ack",
            "status": "error",
            "error": "server_mismatch"
        })
        .to_string();
        quic_write_frame(&mut send, QuicFrameType::Text, error_ack.as_bytes()).await?;
        let _ = send.finish();
        return Ok(());
    }

    let server_audience = claims
        .server_audience
        .as_deref()
        .unwrap_or(&claims.server_id);
    let audience_uuid =
        Uuid::parse_str(server_audience).context("server_audience is not a UUID")?;
    if audience_uuid != server_id {
        warn!(
            "Relay token server_audience {} did not match requested server {}",
            audience_uuid, server_id
        );
        let error_ack = serde_json::json!({
            "type": "connect_ack",
            "status": "error",
            "error": "server_mismatch"
        })
        .to_string();
        quic_write_frame(&mut send, QuicFrameType::Text, error_ack.as_bytes()).await?;
        let _ = send.finish();
        return Ok(());
    }

    if !claims.permissions.iter().any(|p| p == "connect") {
        trace!("Relay token missing connect permission");
        let error_ack = serde_json::json!({
            "type": "connect_ack",
            "status": "error",
            "error": "permission_denied"
        })
        .to_string();
        quic_write_frame(&mut send, QuicFrameType::Text, error_ack.as_bytes()).await?;
        let _ = send.finish();
        return Ok(());
    }

    let tunnel_id = Uuid::new_v4();
    let expires_at = SystemTime::now()
        .checked_add(state.handshake_timeout)
        .and_then(|deadline| deadline.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|dur| dur.as_secs())
        .unwrap_or(0);

    let (tunnel_entry, server_secret) = state.create_tunnel(tunnel_id, Instant::now()).await;
    let mut tunnel_guard = TunnelCleanupGuard::new(state.clone(), tunnel_id);

    if server_entry
        .control_tx
        .send(ServerCommand::OpenTunnel {
            tunnel_id,
            client_id,
            preferred_protocol: "binary",
            expires_at,
            server_secret: server_secret.clone(),
        })
        .await
        .is_err()
    {
        warn!("Failed to notify server {server_id} about new tunnel");
        let error_ack = serde_json::json!({
            "type": "connect_ack",
            "status": "error",
            "error": "server_unreachable"
        })
        .to_string();
        quic_write_frame(&mut send, QuicFrameType::Text, error_ack.as_bytes()).await?;
        let _ = send.finish();
        state.remove_tunnel(&tunnel_id).await;
        tunnel_guard.disarm();
        return Ok(());
    }

    let ack = ConnectAck {
        r#type: "connect_ack",
        status: "ok",
        tunnel_id: tunnel_id.to_string(),
        relay_host: state.relay_host.clone(),
        expires_at,
        server_authority: server_entry.tls_authority.clone(),
    };
    let ack_json = serde_json::to_string(&ack)?;
    quic_write_frame(&mut send, QuicFrameType::Text, ack_json.as_bytes()).await?;

    let ready_frame = loop {
        match quic_read_frame(&mut recv).await? {
            Some(QuicFrame::Text(payload)) => break payload,
            Some(QuicFrame::Ping(payload)) => {
                quic_write_frame(&mut send, QuicFrameType::Pong, payload.as_slice()).await?;
            }
            Some(QuicFrame::Pong(payload)) => {
                trace!(
                    tunnel = %tunnel_id,
                    size = payload.len(),
                    "Received QUIC pong frame during handshake"
                );
            }
            Some(QuicFrame::Binary(_)) => {
                warn!("Client sent unexpected binary frame before tunnel_ready");
            }
            Some(QuicFrame::Close) | None => {
                bail!("client closed before tunnel_ready");
            }
        }
    };

    let ready: TunnelReadyPayload =
        serde_json::from_str(&ready_frame).context("Failed to parse tunnel_ready")?;

    if ready.r#type != "tunnel_ready"
        || ready.role != "client"
        || ready.tunnel_id != tunnel_id.to_string()
    {
        bail!("invalid tunnel_ready payload from client");
    }

    {
        let mut guard = tunnel_entry.state.lock().await;
        guard.mark_client_ready(Instant::now());
    }
    tunnel_entry.notify.notify_waiters();
    trace!("Client readiness recorded for tunnel {}", tunnel_id);

    let endpoint = QuicTunnelEndpoint::new(send, recv);

    if let Some(pair) = tunnel_entry.attach_client_quic(endpoint).await {
        info!("Tunnel {} became active immediately", tunnel_id);
        tunnel_guard.disarm();
        spawn_forwarders(state.clone(), tunnel_id, pair);
        return Ok(());
    }

    let handle_wait = tunnel_entry.clone();
    let handle_cleanup = tunnel_entry.clone();
    let state_clone = state.clone();
    let wait_result = timeout(state.handshake_timeout, async move {
        loop {
            handle_wait.notify.notified().await;

            if let Some(pair) = handle_wait.take_pair_if_ready().await {
                info!("Tunnel {} is now active", tunnel_id);
                spawn_forwarders(state_clone.clone(), tunnel_id, pair);
                break;
            }

            let already_active = {
                let guard = handle_wait.state.lock().await;
                guard.is_fully_ready()
            };

            if already_active {
                trace!(
                    "Tunnel {} already active; connect handler exiting",
                    tunnel_id
                );
                break;
            }
        }
    })
    .await;

    if wait_result.is_err() {
        let already_ready = {
            let guard = tunnel_entry.state.lock().await;
            guard.is_fully_ready()
        };

        if already_ready {
            trace!("Tunnel {} became active before timeout elapsed", tunnel_id);
        } else {
            warn!(
                "Tunnel {} expired waiting for server (client side)",
                tunnel_id
            );
            if let Some(mut endpoint) = handle_cleanup.take_client_endpoint().await {
                let _ = notify_tunnel_failed(&mut endpoint, "timeout").await;
            }
            state.remove_tunnel(&tunnel_id).await;
            tunnel_guard.disarm();
        }
    } else {
        tunnel_guard.disarm();
    }

    Ok(())
}

async fn handle_quic_server_tunnel(
    state: Arc<AppState>,
    payload: TunnelReadyPayload,
    send: quinn::SendStream,
    recv: quinn::RecvStream,
) -> Result<()> {
    if payload.r#type != "tunnel_ready" || payload.role != "server" {
        bail!("Unexpected QUIC tunnel payload type {}", payload.r#type);
    }

    let tunnel_id = Uuid::parse_str(&payload.tunnel_id).context("invalid tunnel_id")?;
    trace!("Server tunnel QUIC attachment for {}", tunnel_id);

    let Some(entry) = state.get_tunnel(&tunnel_id).await else {
        warn!("Received tunnel for unknown id {}", tunnel_id);
        return Ok(());
    };

    let token = payload
        .token
        .as_ref()
        .ok_or_else(|| anyhow!("Server tunnel attach missing authentication token"))?;

    if !entry.verify_server_secret(token).await {
        warn!(
            "Tunnel {} received invalid server authentication token",
            tunnel_id
        );
        return Ok(());
    }

    {
        let mut guard = entry.state.lock().await;
        guard.mark_server_ready(Instant::now());
    }
    entry.notify.notify_waiters();
    trace!("Server readiness recorded for tunnel {}", tunnel_id);

    let endpoint = QuicTunnelEndpoint::new(send, recv);

    if let Some(pair) = entry.attach_server_quic(endpoint).await {
        trace!("Attached server QUIC endpoint; tunnel active immediately");
        spawn_forwarders(state.clone(), tunnel_id, pair);
        return Ok(());
    }

    let handle_wait = entry.clone();
    let handle_cleanup = entry.clone();
    let state_clone = state.clone();
    let wait_result = timeout(state.handshake_timeout, async move {
        loop {
            handle_wait.notify.notified().await;

            if let Some(pair) = handle_wait.take_pair_if_ready().await {
                spawn_forwarders(state_clone.clone(), tunnel_id, pair);
                break;
            }

            let already_active = {
                let guard = handle_wait.state.lock().await;
                guard.is_fully_ready()
            };

            if already_active {
                trace!(
                    "Tunnel {} already active; server QUIC handler exiting",
                    tunnel_id
                );
                break;
            }
        }
    })
    .await;

    if wait_result.is_err() {
        let already_ready = {
            let guard = entry.state.lock().await;
            guard.is_fully_ready()
        };

        if already_ready {
            trace!("Tunnel {} became active before timeout elapsed", tunnel_id);
        } else {
            warn!(
                "Tunnel {} expired waiting for client (server side)",
                tunnel_id
            );
            if let Some(mut server_endpoint) = handle_cleanup.take_server_endpoint().await {
                let _ = close_server_endpoint(&mut server_endpoint).await;
            }
            state.remove_tunnel(&tunnel_id).await;
        }
    }

    Ok(())
}

#[derive(Clone)]
struct AppState {
    bind_address: String,
    listen_addr: String,
    listen_port: u16,
    relay_host: String,
    allow_all_audiences: bool,
    secrets: Arc<HashMap<Uuid, String>>,
    registered_servers: Arc<RwLock<HashMap<Uuid, Arc<RegisteredServer>>>>,
    tunnels: Arc<RwLock<HashMap<Uuid, Arc<TunnelHandle>>>>,
    handshake_timeout: Duration,
    quic_port: Option<u16>,
}

struct RegisteredServer {
    registration_id: Uuid,
    control_tx: mpsc::Sender<ServerCommand>,
    decoding_key: Arc<DecodingKey>,
    tls_authority: Option<String>,
}

enum ServerCommand {
    OpenTunnel {
        tunnel_id: Uuid,
        client_id: Uuid,
        preferred_protocol: &'static str,
        expires_at: u64,
        server_secret: String,
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
                server_secret,
            } => {
                let payload = serde_json::json!({
                    "type": "open_tunnel",
                    "tunnel_id": tunnel_id.to_string(),
                    "client_id": client_id.to_string(),
                    "preferred_protocol": preferred_protocol,
                    "expires_at": expires_at,
                    "server_secret": server_secret,
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
    client_quic: Mutex<Option<QuicTunnelEndpoint>>,
    server_quic: Mutex<Option<QuicTunnelEndpoint>>,
    server_secret: Mutex<Option<String>>,
    notify: Notify,
}

impl TunnelHandle {
    fn new(now: Instant, server_secret: String) -> Self {
        Self {
            state: Mutex::new(TunnelState::new(now)),
            client_ws: Mutex::new(None),
            server_ws: Mutex::new(None),
            client_quic: Mutex::new(None),
            server_quic: Mutex::new(None),
            server_secret: Mutex::new(Some(server_secret)),
            notify: Notify::new(),
        }
    }

    async fn attach_client_ws(&self, ws: WebSocket) -> Option<TunnelPair> {
        {
            let mut guard = self.client_ws.lock().await;
            *guard = Some(ws);
        }
        self.notify.notify_waiters();
        self.take_pair_if_ready().await
    }

    async fn attach_server_ws(&self, ws: WebSocket) -> Option<TunnelPair> {
        {
            let mut guard = self.server_ws.lock().await;
            *guard = Some(ws);
        }
        self.notify.notify_waiters();
        self.take_pair_if_ready().await
    }

    async fn attach_client_quic(&self, endpoint: QuicTunnelEndpoint) -> Option<TunnelPair> {
        {
            let mut guard = self.client_quic.lock().await;
            *guard = Some(endpoint);
        }
        self.notify.notify_waiters();
        self.take_pair_if_ready().await
    }

    async fn attach_server_quic(&self, endpoint: QuicTunnelEndpoint) -> Option<TunnelPair> {
        {
            let mut guard = self.server_quic.lock().await;
            *guard = Some(endpoint);
        }
        self.notify.notify_waiters();
        self.take_pair_if_ready().await
    }

    async fn take_pair_if_ready(&self) -> Option<TunnelPair> {
        // Prefer WebSocket pairing if both sides are present.
        if let Some(client_ws) = self.client_ws.lock().await.take() {
            if let Some(server_ws) = self.server_ws.lock().await.take() {
                return Some(TunnelPair::WebSocket(client_ws, server_ws));
            } else {
                let mut guard = self.client_ws.lock().await;
                *guard = Some(client_ws);
            }
        }

        if let Some(client_quic) = self.client_quic.lock().await.take() {
            if let Some(server_quic) = self.server_quic.lock().await.take() {
                return Some(TunnelPair::Quic(client_quic, server_quic));
            } else {
                let mut guard = self.client_quic.lock().await;
                *guard = Some(client_quic);
            }
        }

        None
    }

    async fn take_client_endpoint(&self) -> Option<TunnelEndpoint> {
        if let Some(ws) = self.client_ws.lock().await.take() {
            return Some(TunnelEndpoint::WebSocket(ws));
        }
        if let Some(quic) = self.client_quic.lock().await.take() {
            return Some(TunnelEndpoint::Quic(quic));
        }
        None
    }

    async fn take_server_endpoint(&self) -> Option<TunnelEndpoint> {
        if let Some(ws) = self.server_ws.lock().await.take() {
            return Some(TunnelEndpoint::WebSocket(ws));
        }
        if let Some(quic) = self.server_quic.lock().await.take() {
            return Some(TunnelEndpoint::Quic(quic));
        }
        None
    }

    async fn verify_server_secret(&self, provided: &str) -> bool {
        let mut guard = self.server_secret.lock().await;
        if let Some(expected) = guard.as_ref() {
            if ConstantTimeEq::ct_eq(expected.as_bytes(), provided.as_bytes()).into() {
                guard.take();
                return true;
            }
        }
        false
    }
}

enum TunnelPair {
    WebSocket(WebSocket, WebSocket),
    Quic(QuicTunnelEndpoint, QuicTunnelEndpoint),
}

enum TunnelEndpoint {
    WebSocket(WebSocket),
    Quic(QuicTunnelEndpoint),
}

#[derive(Debug)]
struct QuicTunnelEndpoint {
    send: Arc<Mutex<quinn::SendStream>>,
    recv: Mutex<Option<quinn::RecvStream>>,
}

impl QuicTunnelEndpoint {
    fn new(send: quinn::SendStream, recv: quinn::RecvStream) -> Self {
        Self {
            send: Arc::new(Mutex::new(send)),
            recv: Mutex::new(Some(recv)),
        }
    }

    async fn take_stream(&self) -> Option<quinn::RecvStream> {
        self.recv.lock().await.take()
    }

    fn send_handle(&self) -> Arc<Mutex<quinn::SendStream>> {
        Arc::clone(&self.send)
    }
}

struct TunnelCleanupGuard {
    state: Arc<AppState>,
    tunnel_id: Uuid,
    disarmed: bool,
}

impl TunnelCleanupGuard {
    fn new(state: Arc<AppState>, tunnel_id: Uuid) -> Self {
        Self {
            state,
            tunnel_id,
            disarmed: false,
        }
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for TunnelCleanupGuard {
    fn drop(&mut self) {
        if self.disarmed {
            return;
        }
        let state = self.state.clone();
        let tunnel_id = self.tunnel_id;
        tokio::spawn(async move {
            state.remove_tunnel(&tunnel_id).await;
        });
    }
}

impl AppState {
    fn new(config: RelayConfig) -> Self {
        let RelayConfig {
            bind_address,
            port,
            handshake_timeout_seconds,
            public_hostname,
            registration_secrets,
            tls_cert_path: _,
            tls_key_path: _,
            quic_port,
            allow_all_audiences,
        } = config;

        let listen_addr = match bind_address.parse::<IpAddr>() {
            Ok(IpAddr::V6(_)) => format!("[{}]:{}", bind_address, port),
            Ok(_) => format!("{}:{}", bind_address, port),
            Err(_) => {
                if bind_address.contains(':') && !bind_address.contains('[') {
                    format!("[{}]:{}", bind_address, port)
                } else {
                    format!("{}:{}", bind_address, port)
                }
            }
        };
        let relay_host = public_hostname
            .and_then(|host| {
                let trimmed = host.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
            .or_else(detect_default_route_ip)
            .unwrap_or_else(|| fallback_public_host(&bind_address));
        let handshake_timeout = Duration::from_secs(handshake_timeout_seconds.max(1));

        Self {
            bind_address,
            listen_addr,
            listen_port: port,
            relay_host,
            allow_all_audiences,
            secrets: Arc::new(registration_secrets),
            registered_servers: Arc::new(RwLock::new(HashMap::new())),
            tunnels: Arc::new(RwLock::new(HashMap::new())),
            handshake_timeout,
            quic_port,
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

    pub(crate) async fn remove_server_if_current(&self, server_id: &Uuid, registration_id: &Uuid) {
        let mut guard = self.registered_servers.write().await;
        let should_remove = guard
            .get(server_id)
            .map(|entry| &entry.registration_id == registration_id)
            .unwrap_or(false);
        if should_remove {
            guard.remove(server_id);
        }
    }

    async fn server_entry(&self, server_id: &Uuid) -> Option<Arc<RegisteredServer>> {
        self.registered_servers.read().await.get(server_id).cloned()
    }

    async fn create_tunnel(&self, tunnel_id: Uuid, now: Instant) -> (Arc<TunnelHandle>, String) {
        let mut secret_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut secret_bytes);
        let server_secret = Base64.encode(secret_bytes);

        let entry = Arc::new(TunnelHandle::new(now, server_secret.clone()));
        self.tunnels.write().await.insert(tunnel_id, entry.clone());
        (entry, server_secret)
    }

    async fn get_tunnel(&self, tunnel_id: &Uuid) -> Option<Arc<TunnelHandle>> {
        self.tunnels.read().await.get(tunnel_id).cloned()
    }

    async fn remove_tunnel(&self, tunnel_id: &Uuid) {
        self.tunnels.write().await.remove(tunnel_id);
    }

    fn is_allowed_audience(&self, audience: &str) -> bool {
        if self.allow_all_audiences {
            return true;
        }

        let aud = audience.trim();
        if aud.eq_ignore_ascii_case(&self.relay_host) {
            return true;
        }

        let host_with_port = format!("{}:{}", self.relay_host, self.listen_port);
        if aud.eq_ignore_ascii_case(&host_with_port) {
            return true;
        }

        if let Ok(url) = Url::parse(aud) {
            if let Some(host) = url.host_str() {
                if host.eq_ignore_ascii_case(&self.relay_host) {
                    if let Some(port) = url.port() {
                        return port == self.listen_port;
                    }
                    return true;
                }
            }
        }

        false
    }
}

fn detect_default_route_ip() -> Option<String> {
    detect_default_route_ip_v6().or_else(detect_default_route_ip_v4)
}

fn detect_default_route_ip_v4() -> Option<String> {
    let socket = UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)).ok()?;
    socket
        .connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 80))
        .ok()?;
    let local_addr = socket.local_addr().ok()?;
    let ip = local_addr.ip();
    if ip.is_unspecified() {
        None
    } else {
        Some(ip.to_string())
    }
}

fn detect_default_route_ip_v6() -> Option<String> {
    let socket = UdpSocket::bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0)).ok()?;
    socket
        .connect(SocketAddr::new(
            IpAddr::V6(Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888)),
            80,
        ))
        .ok()?;
    let local_addr = socket.local_addr().ok()?;
    let ip = local_addr.ip();
    if ip.is_unspecified() {
        None
    } else {
        Some(ip.to_string())
    }
}

fn fallback_public_host(bind_address: &str) -> String {
    match bind_address.parse::<IpAddr>() {
        Ok(ip) if ip.is_unspecified() => match ip {
            IpAddr::V4(_) => "127.0.0.1".to_string(),
            IpAddr::V6(_) => "::1".to_string(),
        },
        Ok(ip) => ip.to_string(),
        Err(_) => bind_address.to_string(),
    }
}

async fn register_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.protocols([RELAY_SUBPROTOCOL])
        .on_failed_upgrade(|error| warn!("register upgrade failed: {error}"))
        .on_upgrade(move |socket| async move {
            if let Err(err) = handle_register_socket(socket, state).await {
                warn!("Register socket ended with error: {err:?}");
            }
        })
}

async fn connect_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.protocols([RELAY_SUBPROTOCOL])
        .on_failed_upgrade(|error| warn!("connect upgrade failed: {error}"))
        .on_upgrade(move |socket| async move {
            if let Err(err) = handle_connect_socket(socket, state).await {
                warn!("Connect socket ended with error: {err:?}");
            }
        })
}

async fn tunnel_handler(
    ws: WebSocketUpgrade,
    Path(tunnel_id): Path<String>,
    Query(query): Query<TunnelQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.protocols([RELAY_SUBPROTOCOL])
        .on_failed_upgrade(|error| warn!("tunnel upgrade failed: {error}"))
        .on_upgrade(move |socket| async move {
            match Uuid::parse_str(&tunnel_id) {
                Ok(tunnel_uuid) => {
                    if let Err(err) =
                        handle_tunnel_socket(socket, state.clone(), tunnel_uuid, query).await
                    {
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
    #[serde(default)]
    tls_authority: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    server_authority: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct TunnelReadyPayload {
    #[serde(rename = "type")]
    r#type: String,
    tunnel_id: String,
    role: String,
    #[serde(default)]
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TunnelQuery {
    role: Option<String>,
    token: Option<String>,
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
    #[serde(default)]
    server_audience: Option<String>,
    #[serde(default)]
    binding_type: Option<String>,
    #[serde(default)]
    binding_value: Option<String>,
    #[serde(default)]
    permissions: Vec<String>,
}

#[instrument(
    level = "trace",
    skip(socket, state),
    fields(server_id = tracing::field::Empty)
)]
async fn handle_register_socket(mut socket: WebSocket, state: Arc<AppState>) -> Result<()> {
    trace!("Register control socket established");
    let Some(Ok(Message::Text(payload))) = socket.recv().await else {
        anyhow::bail!("register socket closed before payload");
    };

    let msg: RegisterPayload =
        serde_json::from_str(&payload).context("Failed to parse register payload")?;
    trace!("Received register payload");
    info!("Register payload tls_authority={:?}", msg.tls_authority);

    if msg.r#type != "register" {
        anyhow::bail!("unexpected message type {}", msg.r#type);
    }

    let server_id = Uuid::parse_str(&msg.server_id).context("invalid server_id")?;
    Span::current().record("server_id", &field::display(&server_id));

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

    // Validate and decode the public key (standard base64-encoded 32 bytes for Ed25519)
    let raw_key = Base64
        .decode(msg.public_key.as_bytes())
        .context("public_key is not valid base64")?;
    let key_bytes: [u8; 32] = raw_key
        .try_into()
        .map_err(|_| anyhow!("public_key must be 32 bytes (Ed25519)"))?;

    // Convert to base64url for from_ed_components (which expects base64url encoding)
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let public_key_b64url = URL_SAFE_NO_PAD.encode(&key_bytes);
    let decoding_key = Arc::new(
        DecodingKey::from_ed_components(&public_key_b64url)
            .context("failed to create decoding key from public key")?,
    );

    info!(
        "Registered server {} with capabilities {:?}",
        server_id, msg.capabilities
    );

    let tls_authority = msg
        .tls_authority
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    let ack = RegisterAck {
        r#type: "register_ack",
        status: "ok",
        retry_after_seconds: 0,
    };

    socket
        .send(Message::Text(serde_json::to_string(&ack)?))
        .await?;
    trace!("Sent register acknowledgement");

    let (tx, mut rx) = mpsc::channel(32);
    let registration_id = Uuid::new_v4();
    let server_entry = Arc::new(RegisteredServer {
        registration_id,
        control_tx: tx.clone(),
        decoding_key,
        tls_authority,
    });

    state.upsert_server(server_id, server_entry.clone()).await;

    loop {
        tokio::select! {
            maybe_cmd = rx.recv() => {
                match maybe_cmd {
                    Some(cmd) => {
                        trace!("Dispatching control command to server");
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
                        trace!("Control channel ping handled");
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {
                        trace!("Ignoring non-text control channel frame");
                    }
                    Some(Err(err)) => {
                        warn!("Control channel read failed for {server_id}: {err}");
                        break;
                    }
                }
            }
        }
    }

    state
        .remove_server_if_current(&server_id, &registration_id)
        .await;
    trace!("Removed server registration");
    Ok(())
}

#[instrument(
    level = "trace",
    skip(socket, state),
    fields(
        server_id = tracing::field::Empty,
        client_id = tracing::field::Empty,
        tunnel_id = tracing::field::Empty
    )
)]
async fn handle_connect_socket(mut socket: WebSocket, state: Arc<AppState>) -> Result<()> {
    trace!("Client connect socket established");
    let Some(Ok(Message::Text(payload))) = socket.recv().await else {
        anyhow::bail!("connect socket closed before payload");
    };

    let msg: ConnectPayload =
        serde_json::from_str(&payload).context("Failed to parse connect payload")?;
    trace!("Received connect payload");

    if msg.r#type != "connect" {
        anyhow::bail!("unexpected message type {}", msg.r#type);
    }

    let server_id = Uuid::parse_str(&msg.server_id).context("invalid server_id")?;
    let client_id = Uuid::parse_str(&msg.client_id).context("invalid client_id")?;
    let client_id_str = client_id.to_string();
    Span::current().record("server_id", &field::display(&server_id));
    Span::current().record("client_id", &field::display(&client_id));
    debug!("Valid connect request parsed");

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
    validation.validate_aud = false;
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

    trace!("Relay token validated");

    if !state.is_allowed_audience(&claims.aud) {
        warn!(
            "Relay token audience '{}' did not match expected host '{}'",
            claims.aud, state.relay_host
        );
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

    let binding_type = claims.binding_type.as_deref().unwrap_or("client_id");
    let binding_value = claims.binding_value.as_deref().unwrap_or(&claims.sub);

    if claims.sub != binding_value {
        warn!(
            "Relay token subject '{}' mismatch binding value '{}'",
            claims.sub, binding_value
        );
        let _ = socket
            .send(Message::Text(
                serde_json::json!({
                    "type": "connect_ack",
                    "status": "error",
                    "error": "binding_mismatch"
                })
                .to_string(),
            ))
            .await;
        let _ = socket.close().await;
        return Ok(());
    }

    if claims.sub != client_id_str {
        warn!(
            "Relay token subject {} did not match requested client {}",
            claims.sub, client_id
        );
        let _ = socket
            .send(Message::Text(
                serde_json::json!({
                    "type": "connect_ack",
                    "status": "error",
                    "error": "client_mismatch"
                })
                .to_string(),
            ))
            .await;
        let _ = socket.close().await;
        return Ok(());
    }

    match binding_type {
        "client_id" => {
            if binding_value != client_id_str {
                warn!(
                    "Relay token binding value '{}' did not match client {}",
                    binding_value, client_id
                );
                let _ = socket
                    .send(Message::Text(
                        serde_json::json!({
                            "type": "connect_ack",
                            "status": "error",
                            "error": "binding_mismatch"
                        })
                        .to_string(),
                    ))
                    .await;
                let _ = socket.close().await;
                return Ok(());
            }
        }
        "enrollment_token" => {
            if binding_value != client_id_str {
                warn!(
                    "Relay token enrollment binding '{}' did not match client {}",
                    binding_value, client_id
                );
                let _ = socket
                    .send(Message::Text(
                        serde_json::json!({
                            "type": "connect_ack",
                            "status": "error",
                            "error": "binding_mismatch"
                        })
                        .to_string(),
                    ))
                    .await;
                let _ = socket.close().await;
                return Ok(());
            }
        }
        other => {
            warn!("Unsupported relay token binding type '{}'", other);
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
    }

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

    let server_audience = claims
        .server_audience
        .as_deref()
        .unwrap_or(&claims.server_id);
    let audience_uuid = match Uuid::parse_str(server_audience) {
        Ok(uuid) => uuid,
        Err(_) => {
            warn!(
                "Relay token server_audience '{}' is not a UUID",
                server_audience
            );
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
    if audience_uuid != server_id {
        warn!(
            "Relay token server_audience {} did not match requested server {}",
            audience_uuid, server_id
        );
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

    let binding_fingerprint = {
        let digest = Sha256::digest(binding_value.as_bytes());
        let hex = hex::encode(digest);
        hex.chars().take(16).collect::<String>()
    };
    debug!(
        server_id = %server_id,
        client_id = %client_id,
        relay_binding_type = binding_type,
        relay_binding_hash = %binding_fingerprint,
        "Relay token binding validated"
    );

    if !claims.permissions.iter().any(|p| p == "connect") {
        trace!("Relay token missing connect permission");
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
    let (tunnel_entry, server_secret) = state.create_tunnel(tunnel_id, Instant::now()).await;
    let mut tunnel_guard = TunnelCleanupGuard::new(state.clone(), tunnel_id);
    Span::current().record("tunnel_id", &field::display(&tunnel_id));
    info!(
        "Issued tunnel {} for client {} via server {}",
        tunnel_id, client_id, server_id
    );

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
            server_secret: server_secret.clone(),
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
        tunnel_guard.disarm();
        return Ok(());
    }

    let ack = ConnectAck {
        r#type: "connect_ack",
        status: "ok",
        tunnel_id: tunnel_id.to_string(),
        relay_host: state.relay_host.clone(),
        expires_at,
        server_authority: server_entry.tls_authority.clone(),
    };

    if server_entry.tls_authority.is_none() {
        warn!(
            "No TLS authority registered for server {server_id}; clients must fall back to local default"
        );
    }

    socket
        .send(Message::Text(serde_json::to_string(&ack)?))
        .await?;
    trace!("Sent connect acknowledgement to client");

    // Wait for tunnel_ready from client
    let Some(Ok(Message::Text(ready_payload))) = socket.recv().await else {
        anyhow::bail!("client closed before tunnel_ready");
    };

    let ready: TunnelReadyPayload =
        serde_json::from_str(&ready_payload).context("Failed to parse tunnel_ready")?;

    if ready.r#type != "tunnel_ready"
        || ready.role != "client"
        || ready.tunnel_id != tunnel_id.to_string()
    {
        anyhow::bail!("invalid tunnel_ready payload from client");
    }

    {
        let mut guard = tunnel_entry.state.lock().await;
        guard.mark_client_ready(Instant::now());
    }
    tunnel_entry.notify.notify_waiters();
    trace!("Client readiness recorded");

    info!(
        "Client {} waiting for server tunnel {}",
        client_id, tunnel_id
    );

    if let Some(pair) = tunnel_entry.attach_client_ws(socket).await {
        info!("Tunnel {} became active immediately", tunnel_id);
        tunnel_guard.disarm();
        spawn_forwarders(state.clone(), tunnel_id, pair);
        return Ok(());
    }

    let handle_wait = tunnel_entry.clone();
    let handle_cleanup = tunnel_entry.clone();
    let state_clone = state.clone();
    let wait_result = timeout(state.handshake_timeout, async move {
        loop {
            handle_wait.notify.notified().await;

            if let Some(pair) = handle_wait.take_pair_if_ready().await {
                info!("Tunnel {} is now active", tunnel_id);
                spawn_forwarders(state_clone.clone(), tunnel_id, pair);
                break;
            }

            let already_active = {
                let guard = handle_wait.state.lock().await;
                guard.is_fully_ready()
            };

            if already_active {
                trace!(
                    "Tunnel {} already active; connect handler exiting",
                    tunnel_id
                );
                break;
            }

            trace!("Notified but tunnel pair not ready yet");
        }
    })
    .await;

    if wait_result.is_err() {
        let already_ready = {
            let guard = tunnel_entry.state.lock().await;
            guard.is_fully_ready()
        };

        if already_ready {
            trace!("Tunnel {} became active before timeout elapsed", tunnel_id);
        } else {
            warn!("Tunnel {} expired waiting for server", tunnel_id);
            if let Some(mut endpoint) = handle_cleanup.take_client_endpoint().await {
                let _ = notify_tunnel_failed(&mut endpoint, "timeout").await;
            }
            state.remove_tunnel(&tunnel_id).await;
            tunnel_guard.disarm();
        }
    }

    let became_active = {
        let guard = tunnel_entry.state.lock().await;
        guard.is_fully_ready()
    };
    if became_active {
        tunnel_guard.disarm();
    }

    Ok(())
}

#[instrument(level = "trace", skip(socket, state))]
async fn handle_tunnel_socket(
    mut socket: WebSocket,
    state: Arc<AppState>,
    tunnel_id: Uuid,
    query: TunnelQuery,
) -> Result<()> {
    trace!("Server tunnel socket established");
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

    if query.role.as_deref() != Some("server") {
        warn!(
            "Tunnel {} missing or invalid role query parameter",
            tunnel_id
        );
        let _ = socket
            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                code: u16::from(CloseCode::Policy),
                reason: "invalid_role".into(),
            })))
            .await;
        state.remove_tunnel(&tunnel_id).await;
        return Ok(());
    }

    let Some(token) = query.token.as_ref() else {
        warn!("Tunnel {} missing server authentication token", tunnel_id);
        let _ = socket
            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                code: u16::from(CloseCode::Policy),
                reason: "missing_token".into(),
            })))
            .await;
        state.remove_tunnel(&tunnel_id).await;
        return Ok(());
    };

    if !entry.verify_server_secret(token).await {
        warn!(
            "Tunnel {} received invalid server authentication token",
            tunnel_id
        );
        let _ = socket
            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                code: u16::from(CloseCode::Policy),
                reason: "invalid_token".into(),
            })))
            .await;
        state.remove_tunnel(&tunnel_id).await;
        return Ok(());
    }

    let Some(Ok(Message::Text(payload))) = socket.recv().await else {
        anyhow::bail!("server tunnel closed before tunnel_ready");
    };

    let ready: TunnelReadyPayload =
        serde_json::from_str(&payload).context("Failed to parse tunnel_ready from server")?;
    trace!("Received server tunnel_ready payload");

    if ready.r#type != "tunnel_ready"
        || ready.role != "server"
        || ready.tunnel_id != tunnel_id.to_string()
    {
        anyhow::bail!("invalid tunnel_ready payload from server");
    }

    {
        let mut guard = entry.state.lock().await;
        guard.mark_server_ready(Instant::now());
    }
    entry.notify.notify_waiters();
    trace!("Server readiness recorded");

    info!("Server confirmed tunnel {}", tunnel_id);

    if let Some(pair) = entry.attach_server_ws(socket).await {
        trace!("Attached server socket; tunnel active immediately");
        spawn_forwarders(state.clone(), tunnel_id, pair);
        return Ok(());
    }

    let handle_wait = entry.clone();
    let handle_cleanup = entry.clone();
    let state_clone = state.clone();
    let wait_result = timeout(state.handshake_timeout, async move {
        loop {
            handle_wait.notify.notified().await;

            if let Some(pair) = handle_wait.take_pair_if_ready().await {
                spawn_forwarders(state_clone.clone(), tunnel_id, pair);
                break;
            }

            let already_active = {
                let guard = handle_wait.state.lock().await;
                guard.is_fully_ready()
            };

            if already_active {
                trace!(
                    "Tunnel {} already active; server handler exiting",
                    tunnel_id
                );
                break;
            }
        }
    })
    .await;

    if wait_result.is_err() {
        let already_ready = {
            let guard = entry.state.lock().await;
            guard.is_fully_ready()
        };

        if already_ready {
            trace!("Tunnel {} became active before timeout elapsed", tunnel_id);
        } else {
            warn!("Tunnel {} expired waiting for client", tunnel_id);
            if let Some(mut server_endpoint) = handle_cleanup.take_server_endpoint().await {
                trace!("Closing server tunnel endpoint after timeout");
                let _ = close_server_endpoint(&mut server_endpoint).await;
            }
            state.remove_tunnel(&tunnel_id).await;
        }
    }

    Ok(())
}

#[instrument(level = "trace", skip(state, pair))]
fn spawn_forwarders(state: Arc<AppState>, tunnel_id: Uuid, pair: TunnelPair) {
    trace!("Spawning forwarders for tunnel {}", tunnel_id);
    match pair {
        TunnelPair::WebSocket(client_ws, server_ws) => {
            tokio::spawn(async move {
                if let Err(err) = forward_bidirectional(tunnel_id, client_ws, server_ws).await {
                    warn!("Tunnel {tunnel_id} forwarding error: {err:?}");
                }
                trace!("Tunnel {tunnel_id} forwarding task finished");
                state.remove_tunnel(&tunnel_id).await;
            });
        }
        TunnelPair::Quic(client_quic, server_quic) => {
            tokio::spawn(async move {
                if let Err(err) =
                    forward_quic_bidirectional(tunnel_id, client_quic, server_quic).await
                {
                    warn!("Tunnel {tunnel_id} QUIC forwarding error: {err:?}");
                }
                trace!("Tunnel {tunnel_id} QUIC forwarding task finished");
                state.remove_tunnel(&tunnel_id).await;
            });
        }
    }
}

#[instrument(level = "trace", skip(client_ws, server_ws))]
async fn forward_bidirectional(
    tunnel_id: Uuid,
    client_ws: WebSocket,
    server_ws: WebSocket,
) -> Result<()> {
    trace!("Starting bidirectional forwarding for tunnel {}", tunnel_id);
    let (client_sink, client_stream) = client_ws.split();
    let (server_sink, server_stream) = server_ws.split();

    let client_to_server =
        tokio::spawn(
            async move { forward_stream("client->server", client_stream, server_sink).await },
        );
    let server_to_client =
        tokio::spawn(
            async move { forward_stream("server->client", server_stream, client_sink).await },
        );

    let (c_res, s_res) = tokio::join!(client_to_server, server_to_client);
    c_res??;
    s_res??;
    trace!("Finished bidirectional forwarding for tunnel {}", tunnel_id);
    Ok(())
}

#[instrument(level = "trace", skip(inbound, outbound))]
async fn forward_stream(
    direction: &'static str,
    mut inbound: SplitStream<WebSocket>,
    mut outbound: SplitSink<WebSocket, Message>,
) -> Result<()> {
    trace!("{direction} stream started");
    while let Some(msg) = inbound.next().await {
        match msg {
            Ok(Message::Binary(bytes)) => {
                trace!("{direction} forwarding {} bytes", bytes.len());
                outbound
                    .send(Message::Binary(bytes))
                    .await
                    .map_err(|err| anyhow!(err))?;
            }
            Ok(Message::Close(frame)) => {
                trace!("{direction} received close frame");
                let _ = outbound.send(Message::Close(frame.clone())).await;
                break;
            }
            Ok(Message::Ping(data)) => {
                trace!("{direction} forwarding ping frame");
                outbound
                    .send(Message::Ping(data))
                    .await
                    .map_err(|err| anyhow!(err))?;
            }
            Ok(Message::Pong(data)) => {
                trace!("{direction} forwarding pong frame");
                outbound
                    .send(Message::Pong(data))
                    .await
                    .map_err(|err| anyhow!(err))?;
            }
            Ok(Message::Text(_)) => {
                // Ignore text frames; relay tunnels only forward binary gRPC frames.
                trace!("{direction} ignoring unexpected text frame");
            }
            Err(err) => return Err(anyhow!(err)),
        }
    }

    let _ = outbound.send(Message::Close(None)).await;
    Ok(())
}

const QUIC_FRAME_HEADER_LEN: usize = 5;

#[repr(u8)]
enum QuicFrameType {
    Text = 0,
    Binary = 1,
    Close = 2,
    Ping = 3,
    Pong = 4,
}

enum QuicFrame {
    Text(String),
    Binary(Vec<u8>),
    Close,
    Ping(Vec<u8>),
    Pong(Vec<u8>),
}

async fn forward_quic_bidirectional(
    tunnel_id: Uuid,
    client: QuicTunnelEndpoint,
    server: QuicTunnelEndpoint,
) -> Result<()> {
    let client_send = client.send_handle();
    let server_send = server.send_handle();
    let client_recv = client
        .take_stream()
        .await
        .context("Client QUIC receive stream unavailable")?;
    let server_recv = server
        .take_stream()
        .await
        .context("Server QUIC receive stream unavailable")?;

    let client_to_server = pipe_quic_stream(
        tunnel_id,
        "client->server",
        client_recv,
        Arc::clone(&server_send),
        Arc::clone(&client_send),
    );
    let server_to_client = pipe_quic_stream(
        tunnel_id,
        "server->client",
        server_recv,
        Arc::clone(&client_send),
        Arc::clone(&server_send),
    );

    tokio::try_join!(client_to_server, server_to_client)?;

    Ok(())
}

async fn pipe_quic_stream(
    tunnel_id: Uuid,
    direction: &'static str,
    mut recv: quinn::RecvStream,
    dest_send: Arc<Mutex<quinn::SendStream>>,
    source_send: Arc<Mutex<quinn::SendStream>>,
) -> Result<()> {
    loop {
        match quic_read_frame(&mut recv).await? {
            Some(QuicFrame::Binary(payload)) => {
                let mut guard = dest_send.lock().await;
                quic_write_frame(&mut *guard, QuicFrameType::Binary, payload.as_slice()).await?;
            }
            Some(QuicFrame::Text(_)) => {
                trace!(
                    tunnel = %tunnel_id,
                    direction = direction,
                    "Ignoring unexpected text frame on QUIC tunnel"
                );
            }
            Some(QuicFrame::Ping(payload)) => {
                let mut guard = source_send.lock().await;
                quic_write_frame(&mut *guard, QuicFrameType::Pong, payload.as_slice()).await?;
            }
            Some(QuicFrame::Pong(payload)) => {
                trace!(
                    tunnel = %tunnel_id,
                    direction = direction,
                    size = payload.len(),
                    "Received QUIC pong frame"
                );
            }
            Some(QuicFrame::Close) | None => break,
        }
    }

    trace!(
        tunnel = %tunnel_id,
        direction = direction,
        "QUIC stream pipeline finished"
    );
    Ok(())
}

async fn quic_read_frame(stream: &mut quinn::RecvStream) -> Result<Option<QuicFrame>> {
    let mut header = [0u8; QUIC_FRAME_HEADER_LEN];
    match stream.read_exact(&mut header).await {
        Ok(()) => {}
        Err(ReadExactError::FinishedEarly(_)) => return Ok(None),
        Err(ReadExactError::ReadError(err)) => {
            return Err(anyhow!("Failed to read QUIC frame header: {err}"));
        }
    }

    let frame_type = match header[0] {
        0 => QuicFrameType::Text,
        1 => QuicFrameType::Binary,
        2 => QuicFrameType::Close,
        3 => QuicFrameType::Ping,
        4 => QuicFrameType::Pong,
        other => bail!("Unknown QUIC frame type {}", other),
    };

    let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    let mut payload = vec![0u8; length];
    if length > 0 {
        match stream.read_exact(&mut payload).await {
            Ok(()) => {}
            Err(ReadExactError::FinishedEarly(_)) => return Ok(None),
            Err(ReadExactError::ReadError(err)) => {
                return Err(anyhow!("Failed to read QUIC frame payload: {err}"));
            }
        }
    }

    let frame = match frame_type {
        QuicFrameType::Text => {
            let text =
                String::from_utf8(payload).context("QUIC control frame contained invalid UTF-8")?;
            QuicFrame::Text(text)
        }
        QuicFrameType::Binary => QuicFrame::Binary(payload),
        QuicFrameType::Close => QuicFrame::Close,
        QuicFrameType::Ping => QuicFrame::Ping(payload),
        QuicFrameType::Pong => QuicFrame::Pong(payload),
    };

    Ok(Some(frame))
}

async fn quic_write_frame(
    stream: &mut quinn::SendStream,
    frame_type: QuicFrameType,
    payload: &[u8],
) -> Result<()> {
    if payload.len() > u32::MAX as usize {
        bail!("QUIC frame payload exceeds u32::MAX");
    }

    let mut header = [0u8; QUIC_FRAME_HEADER_LEN];
    header[0] = frame_type as u8;
    header[1..5].copy_from_slice(&(payload.len() as u32).to_be_bytes());

    stream
        .write_all(&header)
        .await
        .context("Failed to write QUIC frame header")?;
    if !payload.is_empty() {
        stream
            .write_all(payload)
            .await
            .context("Failed to write QUIC frame payload")?;
    }
    Ok(())
}

async fn quic_send_text(endpoint: &QuicTunnelEndpoint, text: &str) -> Result<()> {
    let send = endpoint.send_handle();
    let mut guard = send.lock().await;
    quic_write_frame(&mut *guard, QuicFrameType::Text, text.as_bytes()).await
}

async fn quic_send_close(endpoint: &QuicTunnelEndpoint) -> Result<()> {
    let send = endpoint.send_handle();
    let mut guard = send.lock().await;
    quic_write_frame(&mut *guard, QuicFrameType::Close, &[]).await?;
    guard
        .finish()
        .map_err(|err| anyhow!("Failed to finish QUIC stream: {err}"))
}

async fn notify_tunnel_failed(endpoint: &mut TunnelEndpoint, reason: &str) -> Result<()> {
    let payload = serde_json::json!({
        "type": "tunnel_failed",
        "reason": reason,
    })
    .to_string();

    match endpoint {
        TunnelEndpoint::WebSocket(ws) => {
            let _ = ws.send(Message::Text(payload)).await;
            let _ = ws.close().await;
        }
        TunnelEndpoint::Quic(quic) => {
            let _ = quic_send_text(quic, &payload).await;
            let _ = quic_send_close(quic).await;
        }
    }
    Ok(())
}

async fn close_server_endpoint(endpoint: &mut TunnelEndpoint) -> Result<()> {
    match endpoint {
        TunnelEndpoint::WebSocket(ws) => {
            let _ = ws
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: u16::from(CloseCode::Normal),
                    reason: "timeout".into(),
                })))
                .await;
        }
        TunnelEndpoint::Quic(quic) => {
            let _ = quic_send_close(quic).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RelayConfig;
    use anyhow::{Result, anyhow};
    use ed25519_dalek::{SigningKey, pkcs8::EncodePrivateKey};
    use http::Uri;
    use hyper_util::rt::TokioIo;
    use jsonwebtoken::{Algorithm, EncodingKey, Header};
    use serde::Serialize;
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{mpsc, oneshot},
        time::Duration,
    };
    use tokio_stream::wrappers::TcpListenerStream;
    use tokio_tungstenite::{
        MaybeTlsStream, WebSocketStream, connect_async,
        tungstenite::{Message as WsMessage, client::IntoClientRequest, http::HeaderValue},
    };
    use tonic::{
        Request, Response, Status,
        transport::{Certificate, ClientTlsConfig, Endpoint, Identity, ServerTlsConfig},
    };
    use tower::service_fn;
    use url::form_urlencoded;

    mod proto {
        tonic::include_proto!("relay.test");
    }
    use proto::echo_service_client::EchoServiceClient;
    use proto::echo_service_server::{EchoService, EchoServiceServer};
    use proto::{EchoRequest, EchoResponse};

    #[derive(Serialize)]
    struct TestClaims {
        iss: &'static str,
        sub: String,
        aud: String,
        exp: u64,
        iat: u64,
        server_id: String,
        permissions: Vec<&'static str>,
    }

    #[derive(Clone)]
    struct TestCertificate {
        pem_cert: String,
        pem_key: String,
    }

    fn ws_request(url: &str) -> tokio_tungstenite::tungstenite::handshake::client::Request {
        let mut request = url
            .into_client_request()
            .expect("failed to construct websocket request");
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            HeaderValue::from_static(RELAY_SUBPROTOCOL),
        );
        request
    }

    async fn connect_with_protocol(
        url: &str,
    ) -> tokio_tungstenite::tungstenite::Result<(
        WebSocketStream<MaybeTlsStream<TcpStream>>,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    )> {
        match connect_async(ws_request(url)).await {
            Ok(pair) => Ok(pair),
            Err(err) => {
                eprintln!("connect_with_protocol error for {url}: {err:?}");
                Err(err)
            }
        }
    }

    fn generate_test_certificate() -> TestCertificate {
        let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(vec![
            "handcontrol.local".to_string(),
            "localhost".to_string(),
            "127.0.0.1".to_string(),
        ])
        .expect("failed to generate test certificate");
        let pem_cert = cert.pem();
        let pem_key = signing_key.serialize_pem();
        TestCertificate { pem_cert, pem_key }
    }

    #[derive(Default)]
    struct TestEchoService;

    #[tonic::async_trait]
    impl EchoService for TestEchoService {
        async fn echo(
            &self,
            request: Request<EchoRequest>,
        ) -> Result<Response<EchoResponse>, Status> {
            let message = request.into_inner().message;
            Ok(Response::new(EchoResponse {
                message: format!("echo: {message}"),
            }))
        }
    }

    async fn start_test_grpc_server(
        cert: &TestCertificate,
    ) -> Result<(SocketAddr, oneshot::Sender<()>), anyhow::Error> {
        let identity = Identity::from_pem(cert.pem_cert.clone(), cert.pem_key.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        tokio::spawn(async move {
            let server = EchoServiceServer::new(TestEchoService::default());
            let tls_config = ServerTlsConfig::new().identity(identity);
            let result = tonic::transport::Server::builder()
                .tls_config(tls_config)
                .expect("configure TLS")
                .add_service(server)
                .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async move {
                    let _ = shutdown_rx.await;
                })
                .await;
            if let Err(err) = result {
                tracing::error!("gRPC test server error: {err:?}");
            }
        });

        Ok((addr, shutdown_tx))
    }

    async fn relay_between_ws_and_tcp(
        ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
        tcp_stream: TcpStream,
    ) -> Result<()> {
        let (ws_sink, mut ws_stream) = ws.split();
        let sink = Arc::new(tokio::sync::Mutex::new(ws_sink));
        let (mut tcp_reader, mut tcp_writer) = tcp_stream.into_split();

        let sink_for_stream = sink.clone();
        let ws_to_tcp = async move {
            while let Some(msg) = ws_stream.next().await {
                match msg? {
                    WsMessage::Binary(data) => {
                        tcp_writer.write_all(data.as_slice()).await?;
                    }
                    WsMessage::Close(_) => break,
                    WsMessage::Ping(payload) => {
                        sink_for_stream
                            .lock()
                            .await
                            .send(WsMessage::Pong(payload))
                            .await?;
                    }
                    WsMessage::Pong(_) => {}
                    WsMessage::Text(_) => {}
                    _ => {}
                }
            }
            Ok::<(), anyhow::Error>(())
        };

        let sink_for_tcp = sink.clone();
        let tcp_to_ws = async move {
            let mut buf = [0u8; 16 * 1024];
            loop {
                let read = tcp_reader.read(&mut buf).await?;
                if read == 0 {
                    break;
                }
                sink_for_tcp
                    .lock()
                    .await
                    .send(WsMessage::Binary(buf[..read].to_vec().into()))
                    .await?;
            }
            Ok::<(), anyhow::Error>(())
        };

        tokio::try_join!(ws_to_tcp, tcp_to_ws)?;

        Ok(())
    }

    fn test_config(server_id: Uuid, secret: &str, timeout_secs: u64) -> RelayConfig {
        let mut secrets = HashMap::new();
        secrets.insert(server_id, secret.to_string());
        RelayConfig {
            bind_address: "127.0.0.1".to_string(),
            port: 0,
            handshake_timeout_seconds: timeout_secs,
            public_hostname: Some("127.0.0.1".to_string()),
            registration_secrets: secrets,
            tls_cert_path: None,
            tls_key_path: None,
            quic_port: None,
            allow_all_audiences: false,
        }
    }

    #[test]
    fn ipv6_bind_address_formats_with_brackets() {
        let server_id = Uuid::new_v4();
        let secret = Base64.encode(b"ipv6-secret".as_ref());
        let mut config = test_config(server_id, &secret, 5);
        config.bind_address = "::1".to_string();
        config.port = 8443;
        let state = AppState::new(config);
        assert_eq!(state.listen_addr, "[::1]:8443");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn re_register_does_not_remove_new_entry() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;

        let server_id = Uuid::new_v4();
        let secret = Base64.encode(b"re-register-secret".as_ref());
        let config = test_config(server_id, &secret, 5);
        let state = Arc::new(AppState::new(config));

        let signing_key = SigningKey::from_bytes(&[5u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let public_key_b64url = URL_SAFE_NO_PAD.encode(verifying_key.to_bytes());
        let decoding_key = Arc::new(
            DecodingKey::from_ed_components(&public_key_b64url)
                .expect("failed to construct decoding key"),
        );

        let (tx_old, _rx_old) = mpsc::channel(1);
        let first_entry = Arc::new(RegisteredServer {
            registration_id: Uuid::new_v4(),
            control_tx: tx_old,
            decoding_key: decoding_key.clone(),
            tls_authority: Some("first".to_string()),
        });
        state.upsert_server(server_id, first_entry.clone()).await;

        let (tx_new, _rx_new) = mpsc::channel(1);
        let second_entry = Arc::new(RegisteredServer {
            registration_id: Uuid::new_v4(),
            control_tx: tx_new,
            decoding_key,
            tls_authority: Some("second".to_string()),
        });
        state.upsert_server(server_id, second_entry.clone()).await;

        state
            .remove_server_if_current(&server_id, &first_entry.registration_id)
            .await;
        let current = state
            .server_entry(&server_id)
            .await
            .expect("new registration should remain");
        assert!(
            Arc::ptr_eq(&current, &second_entry),
            "stale control connection removed active registration"
        );

        state
            .remove_server_if_current(&server_id, &second_entry.registration_id)
            .await;
        assert!(
            state.server_entry(&server_id).await.is_none(),
            "active registration should be removable"
        );
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
        let (control_ws, _) = connect_with_protocol(&register_url).await.unwrap();
        let (mut control_sink, mut control_stream) = control_ws.split();

        let register_msg = json!({
            "type": "register",
            "server_id": server_id.to_string(),
            "relay_secret": secret,
            "server_version": "test",
            "capabilities": ["relay.v1"],
            "public_key": public_key_base64,
            "max_tunnels": 4,
            "tls_authority": "handcontrol.local:50051",
        });
        control_sink
            .send(WsMessage::Text(register_msg.to_string().into()))
            .await
            .unwrap();

        let ack_msg = control_stream.next().await.unwrap().unwrap();
        let ack_text = ack_msg.into_text().unwrap().to_string();
        let ack_value: Value = serde_json::from_str(ack_text.as_str()).unwrap();
        assert_eq!(
            ack_value["status"], "ok",
            "connect ack error: {ack_value:?}"
        );
        match &ack_value["server_authority"] {
            Value::String(value) => assert_eq!(value, "handcontrol.local:50051"),
            Value::Null => (),
            other => panic!("unexpected server_authority field: {other:?}"),
        }

        let (open_tunnel_tx, open_tunnel_rx) = oneshot::channel::<(Uuid, String)>();
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
                                if let (Some(tx), Some(tunnel_id_str), Some(secret_str)) = (
                                    open_tunnel_tx.take(),
                                    value["tunnel_id"].as_str(),
                                    value["server_secret"].as_str(),
                                ) {
                                    let tunnel_id = Uuid::parse_str(tunnel_id_str).unwrap();
                                    let _ = tx.send((tunnel_id, secret_str.to_string()));
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
        let (mut client_ws, _) = connect_with_protocol(&connect_url).await.unwrap();

        let client_id = Uuid::new_v4();

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = TestClaims {
            iss: "handcontrol-server",
            sub: client_id.to_string(),
            aud: "127.0.0.1".to_string(),
            exp: now + 60,
            iat: now,
            server_id: server_id.to_string(),
            permissions: vec!["connect"],
        };
        let token =
            jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), &claims, &encoding_key).unwrap();

        // Token validation happens in the relay server; we trust the relay to validate correctly

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
        let ack_value: Value = serde_json::from_str(ack_text.as_str()).unwrap();
        assert_eq!(ack_value["type"], "connect_ack");
        assert_eq!(
            ack_value["status"], "ok",
            "connect ack error: {ack_value:?}"
        );
        match &ack_value["server_authority"] {
            Value::String(value) => assert_eq!(value, "handcontrol.local:50051"),
            Value::Null => (),
            other => panic!("unexpected server_authority field: {other:?}"),
        }
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

        let (tunnel_id_notify, server_secret) = open_tunnel_rx.await.unwrap();
        assert_eq!(tunnel_id, tunnel_id_notify);

        let secret_param: String =
            url::form_urlencoded::byte_serialize(server_secret.as_bytes()).collect();
        let server_tunnel_url =
            format!("ws://{addr}/tunnel/{tunnel_id}?role=server&token={secret_param}");
        let (server_tunnel_ws, _) = connect_with_protocol(&server_tunnel_url).await.unwrap();
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
        control_task.abort(); // Control task runs infinite loop, must abort
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
        let (control_ws, _) = connect_with_protocol(&register_url).await.unwrap();
        let (mut control_sink, mut control_stream) = control_ws.split();

        let register_msg = json!({
            "type": "register",
            "server_id": server_id.to_string(),
            "relay_secret": secret,
            "server_version": "test",
            "capabilities": ["relay.v1"],
            "public_key": public_key_base64,
            "max_tunnels": 1,
            "tls_authority": "handcontrol.local:50051",
        });
        control_sink
            .send(WsMessage::Text(register_msg.to_string().into()))
            .await
            .unwrap();

        let ack_msg = control_stream.next().await.unwrap().unwrap();
        let ack_text = ack_msg.into_text().unwrap().to_string();
        let ack_value: Value = serde_json::from_str(ack_text.as_str()).unwrap();
        assert_eq!(ack_value["status"], "ok");
        match &ack_value["server_authority"] {
            Value::String(value) => assert_eq!(value, "handcontrol.local:50051"),
            Value::Null => (),
            other => panic!("unexpected server_authority field: {other:?}"),
        }

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
        let (mut client_ws, _) = connect_with_protocol(&connect_url).await.unwrap();

        let client_id = Uuid::new_v4();

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = TestClaims {
            iss: "handcontrol-server",
            sub: client_id.to_string(),
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
            "client_id": client_id.to_string(),
            "client_version": "integration-test",
        });
        client_ws
            .send(WsMessage::Text(connect_msg.to_string().into()))
            .await
            .unwrap();

        let ack_msg = client_ws.next().await.unwrap().unwrap();
        let ack_text = ack_msg.into_text().unwrap().to_string();
        let ack_value: Value = serde_json::from_str(ack_text.as_str()).unwrap();
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
        control_task.abort(); // Control task runs infinite loop, must abort
        server_handle.abort();
        let _ = server_handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn connect_rejects_client_mismatch() {
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

        let signing_key = SigningKey::from_bytes(&[11u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let public_key_base64 = Base64.encode(verifying_key.to_bytes());
        let private_der = signing_key.to_pkcs8_der().unwrap();
        let encoding_key = EncodingKey::from_ed_der(private_der.as_bytes());

        // Register server
        let register_url = format!("ws://{addr}/register");
        let (control_ws, _) = connect_with_protocol(&register_url).await.unwrap();
        let (mut control_sink, mut control_stream) = control_ws.split();

        let register_msg = json!({
            "type": "register",
            "server_id": server_id.to_string(),
            "relay_secret": secret,
            "server_version": "test",
            "capabilities": ["relay.v1"],
            "public_key": public_key_base64,
            "max_tunnels": 1,
            "tls_authority": "handcontrol.local:50051",
        });
        control_sink
            .send(WsMessage::Text(register_msg.to_string().into()))
            .await
            .unwrap();

        let ack_msg = control_stream.next().await.unwrap().unwrap();
        let ack_text = ack_msg.into_text().unwrap().to_string();
        let ack_value: Value = serde_json::from_str(ack_text.as_str()).unwrap();
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

        // Client connects with mismatched client_id
        let connect_url = format!("ws://{addr}/connect");
        let (mut client_ws, _) = connect_with_protocol(&connect_url).await.unwrap();

        let client_id_in_token = Uuid::new_v4();
        let mismatched_client_id = Uuid::new_v4();

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = TestClaims {
            iss: "handcontrol-server",
            sub: client_id_in_token.to_string(),
            aud: "127.0.0.1".to_string(),
            exp: now + 60,
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
            "client_id": mismatched_client_id.to_string(),
            "client_version": "integration-test",
        });
        client_ws
            .send(WsMessage::Text(connect_msg.to_string().into()))
            .await
            .unwrap();

        let ack_msg = client_ws.next().await.unwrap().unwrap();
        let ack_text = ack_msg.into_text().unwrap().to_string();
        let ack_value: Value = serde_json::from_str(ack_text.as_str()).unwrap();
        assert_eq!(ack_value["status"], "error");
        assert_eq!(ack_value["error"], "client_mismatch");

        let _ = client_ws.close(None).await;
        control_task.abort();
        server_handle.abort();
        let _ = server_handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 6)]
    async fn grpc_tunnel_forwards_grpc_response() {
        let server_id = Uuid::new_v4();
        let secret = Base64.encode(b"relayed-secret".as_ref());
        let config = test_config(server_id, &secret, 10);
        let state = Arc::new(AppState::new(config));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = listener.local_addr().unwrap();
        let app_state = state.clone();
        let relay_handle = tokio::spawn(async move {
            axum::serve(listener, build_router(app_state).into_make_service())
                .await
                .unwrap();
        });

        let cert = generate_test_certificate();
        let (grpc_addr, grpc_shutdown) = start_test_grpc_server(&cert).await.unwrap();
        let server_authority = format!("handcontrol.local:{}", grpc_addr.port());

        let signing_key = SigningKey::from_bytes(&[13u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let public_key_base64 = Base64.encode(verifying_key.to_bytes());
        let private_der = signing_key.to_pkcs8_der().unwrap();
        let encoding_key = EncodingKey::from_ed_der(private_der.as_bytes());

        // Register server with relay
        let register_url = format!("ws://{relay_addr}/register");
        let (control_ws, _) = connect_with_protocol(&register_url).await.unwrap();
        let (mut control_sink, mut control_stream) = control_ws.split();

        let register_msg = json!({
            "type": "register",
            "server_id": server_id.to_string(),
            "relay_secret": secret,
            "server_version": "test",
            "capabilities": ["relay.v1"],
            "public_key": public_key_base64,
            "max_tunnels": 2,
            "tls_authority": server_authority,
        });
        control_sink
            .send(WsMessage::Text(register_msg.to_string().into()))
            .await
            .unwrap();

        let ack_msg = control_stream.next().await.unwrap().unwrap();
        let ack_text = ack_msg.into_text().unwrap().to_string();
        let ack_value: Value = serde_json::from_str(ack_text.as_str()).unwrap();
        assert_eq!(ack_value["status"], "ok");
        if let Some(authority) = ack_value["server_authority"].as_str() {
            assert_eq!(authority, server_authority);
        }

        let (open_tunnel_tx, open_tunnel_rx) = oneshot::channel::<(Uuid, String)>();
        let control_task = tokio::spawn(async move {
            let mut open_tunnel_tx = Some(open_tunnel_tx);
            while let Some(msg) = control_stream.next().await {
                match msg {
                    Ok(WsMessage::Text(text)) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&text.to_string()) {
                            if value["type"] == "open_tunnel" {
                                if let (Some(tx), Some(tunnel_id), Some(secret)) = (
                                    open_tunnel_tx.take(),
                                    value["tunnel_id"].as_str(),
                                    value["server_secret"].as_str(),
                                ) {
                                    let tunnel_id = Uuid::parse_str(tunnel_id).unwrap();
                                    let _ = tx.send((tunnel_id, secret.to_string()));
                                }
                            }
                        }
                    }
                    Ok(WsMessage::Ping(payload)) => {
                        if control_sink.send(WsMessage::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Ok(WsMessage::Close(_)) | Err(_) => break,
                    _ => {}
                }
            }
        });

        // Client initiates relay tunnel
        let connect_url = format!("ws://{relay_addr}/connect");
        let (mut client_ws, _) = connect_with_protocol(&connect_url).await.unwrap();
        let client_id = Uuid::new_v4();

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = TestClaims {
            iss: "handcontrol-server",
            sub: client_id.to_string(),
            aud: "127.0.0.1".to_string(),
            exp: now + 60,
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
            "client_id": client_id.to_string(),
            "client_version": "integration-test",
        });
        client_ws
            .send(WsMessage::Text(connect_msg.to_string().into()))
            .await
            .unwrap();

        let ack_msg = client_ws.next().await.unwrap().unwrap();
        let ack_text = ack_msg.into_text().unwrap().to_string();
        let ack_value: Value = serde_json::from_str(ack_text.as_str()).unwrap();
        assert_eq!(ack_value["status"], "ok");
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

        let (opened_tunnel_id, server_secret) = open_tunnel_rx.await.unwrap();
        assert_eq!(opened_tunnel_id, tunnel_id);

        let server_tunnel_task = {
            let server_secret = server_secret.clone();
            let relay_addr = relay_addr;
            tokio::spawn(async move {
                let secret_param: String =
                    url::form_urlencoded::byte_serialize(server_secret.as_bytes()).collect();
                let server_tunnel_url = format!(
                    "ws://{relay_addr}/tunnel/{tunnel_id}?role=server&token={secret_param}"
                );
                let (mut server_ws, _) = connect_with_protocol(&server_tunnel_url).await?;
                let ready_msg = json!({
                    "type": "tunnel_ready",
                    "tunnel_id": tunnel_id.to_string(),
                    "role": "server",
                });
                server_ws
                    .send(WsMessage::Text(ready_msg.to_string().into()))
                    .await?;
                let tcp = TcpStream::connect(grpc_addr).await?;
                relay_between_ws_and_tcp(server_ws, tcp).await
            })
        };

        let local_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bridge_addr = local_listener.local_addr().unwrap();
        let client_tunnel_task = tokio::spawn(async move {
            let (socket, _) = local_listener.accept().await?;
            relay_between_ws_and_tcp(client_ws, socket).await
        });

        let client_tls = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(cert.pem_cert.clone()))
            .domain_name("handcontrol.local");
        let endpoint =
            Endpoint::from_shared(format!("https://handcontrol.local:{}", bridge_addr.port()))
                .unwrap()
                .tls_config(client_tls)
                .unwrap();
        let bridge_target = bridge_addr;
        let connector = service_fn(move |_: Uri| {
            let addr = bridge_target;
            async move {
                let stream = TcpStream::connect(addr).await?;
                Ok::<_, std::io::Error>(TokioIo::new(stream))
            }
        });
        let channel = endpoint.connect_with_connector(connector).await.unwrap();
        let mut client = EchoServiceClient::new(channel.clone());

        let response = client
            .echo(Request::new(EchoRequest {
                message: "hello".to_string(),
            }))
            .await
            .unwrap();
        assert_eq!(response.into_inner().message, "echo: hello");

        drop(client);
        drop(channel);

        client_tunnel_task.abort();
        let _ = client_tunnel_task.await;
        server_tunnel_task.abort();
        let _ = server_tunnel_task.await;

        grpc_shutdown.send(()).ok();
        control_task.abort();
        let _ = control_task.await;
        relay_handle.abort();
        let _ = relay_handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 6)]
    async fn control_connection_stays_connected_after_tunnel_close() {
        let server_id = Uuid::new_v4();
        let secret = Base64.encode(b"relayed-secret".as_ref());
        let config = test_config(server_id, &secret, 5);
        let state = Arc::new(AppState::new(config));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = listener.local_addr().unwrap();
        let app_state = state.clone();
        let relay_handle = tokio::spawn(async move {
            axum::serve(listener, build_router(app_state).into_make_service())
                .await
                .unwrap();
        });

        // Prepare signing key for test server (acts like TokenIssuer)
        let signing_key_bytes = [42u8; 32];
        let signing_key = SigningKey::from_bytes(&signing_key_bytes);
        let verifying_key = signing_key.verifying_key();
        let public_key_base64 = Base64.encode(verifying_key.to_bytes());

        let base_ws_url = format!("ws://{}", relay_addr);
        let register_url = format!("{}/register", base_ws_url);

        let (control_closed_tx, control_closed_rx) = oneshot::channel();
        let server_task = tokio::spawn(run_test_server_control(
            register_url,
            base_ws_url.clone(),
            server_id,
            secret.clone(),
            public_key_base64.clone(),
            control_closed_tx,
        ));

        // Wait for server registration to be visible
        let mut attempts = 0;
        loop {
            if state.server_entry(&server_id).await.is_some() {
                break;
            }
            attempts += 1;
            if attempts > 200 {
                panic!("Server did not register with relay");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let client_id = Uuid::new_v4();
        let relay_token = {
            let client_id_str = client_id.to_string();
            create_client_token(
                &signing_key,
                server_id,
                &client_id_str,
                "127.0.0.1",
                "client_id",
                &client_id_str,
            )
        }
        .unwrap();

        let connect_url = format!("{}/connect", base_ws_url);
        let (client_ws, _) = connect_with_protocol(&connect_url).await.unwrap();
        let (mut client_sink, mut client_stream) = client_ws.split();

        let connect_msg = json!({
            "type": "connect",
            "server_id": server_id.to_string(),
            "relay_token": relay_token,
            "client_id": client_id.to_string(),
            "client_version": "integration-test",
        });
        client_sink
            .send(WsMessage::Text(connect_msg.to_string().into()))
            .await
            .unwrap();

        let ack_msg = client_stream.next().await.unwrap().unwrap();
        let ack_text = match ack_msg {
            WsMessage::Text(text) => text,
            other => panic!("Expected text ack, got {:?}", other),
        };
        let ack_value: Value = serde_json::from_str(ack_text.as_str()).unwrap();
        assert_eq!(ack_value["status"], "ok");
        let tunnel_id = ack_value["tunnel_id"]
            .as_str()
            .expect("missing tunnel_id in ack")
            .to_string();

        let ready_msg = json!({
            "type": "tunnel_ready",
            "tunnel_id": tunnel_id,
            "role": "client",
        });
        client_sink
            .send(WsMessage::Text(ready_msg.to_string().into()))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;

        client_sink.send(WsMessage::Close(None)).await.unwrap();
        let _ = client_stream.next().await;

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            _ = control_closed_rx => panic!("Control connection closed unexpectedly"),
        }

        relay_handle.abort();
        let _ = relay_handle.await;
        server_task.abort();
        let _ = server_task.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 6)]
    async fn relay_rejects_binding_mismatch() {
        let server_id = Uuid::new_v4();
        let secret = Base64.encode(b"relayed-secret".as_ref());
        let config = test_config(server_id, &secret, 5);
        let state = Arc::new(AppState::new(config));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = listener.local_addr().unwrap();
        let app_state = state.clone();
        let relay_handle = tokio::spawn(async move {
            axum::serve(listener, build_router(app_state).into_make_service())
                .await
                .unwrap();
        });

        let signing_key_bytes = [33u8; 32];
        let signing_key = SigningKey::from_bytes(&signing_key_bytes);
        let verifying_key = signing_key.verifying_key();
        let public_key_base64 = Base64.encode(verifying_key.to_bytes());

        let base_ws_url = format!("ws://{}", relay_addr);
        let register_url = format!("{}/register", base_ws_url);

        let (control_closed_tx, control_closed_rx) = oneshot::channel();
        let server_task = tokio::spawn(run_test_server_control(
            register_url,
            base_ws_url.clone(),
            server_id,
            secret.clone(),
            public_key_base64.clone(),
            control_closed_tx,
        ));

        let mut attempts = 0;
        loop {
            if state.server_entry(&server_id).await.is_some() {
                break;
            }
            attempts += 1;
            if attempts > 200 {
                panic!("Server did not register with relay");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let client_id = Uuid::new_v4();
        let relay_token = {
            let subject = client_id.to_string();
            let mismatched = Uuid::new_v4().to_string();
            create_client_token(
                &signing_key,
                server_id,
                &subject,
                "127.0.0.1",
                "client_id",
                &mismatched,
            )
        }
        .unwrap();

        let connect_url = format!("{}/connect", base_ws_url);
        let (client_ws, _) = connect_with_protocol(&connect_url).await.unwrap();
        let (mut client_sink, mut client_stream) = client_ws.split();

        let connect_msg = json!({
            "type": "connect",
            "server_id": server_id.to_string(),
            "relay_token": relay_token,
            "client_id": client_id.to_string(),
            "client_version": "integration-test",
        });
        client_sink
            .send(WsMessage::Text(connect_msg.to_string().into()))
            .await
            .unwrap();

        let ack_msg = client_stream.next().await.unwrap().unwrap();
        let ack_text = match ack_msg {
            WsMessage::Text(text) => text,
            other => panic!("Expected text ack, got {:?}", other),
        };
        let ack_value: Value = serde_json::from_str(ack_text.as_str()).unwrap();
        assert_eq!(ack_value["status"], "error");
        assert_eq!(ack_value["error"], "binding_mismatch");

        drop(client_sink);
        drop(client_stream);

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            _ = control_closed_rx => {}
        }

        relay_handle.abort();
        let _ = relay_handle.await;
        server_task.abort();
        let _ = server_task.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 6)]
    async fn control_connection_handles_abrupt_tunnel_close() {
        let server_id = Uuid::new_v4();
        let secret = Base64.encode(b"relayed-secret".as_ref());
        let config = test_config(server_id, &secret, 5);
        let state = Arc::new(AppState::new(config));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = listener.local_addr().unwrap();
        let app_state = state.clone();
        let relay_handle = tokio::spawn(async move {
            axum::serve(listener, build_router(app_state).into_make_service())
                .await
                .unwrap();
        });

        let signing_key_bytes = [84u8; 32];
        let signing_key = SigningKey::from_bytes(&signing_key_bytes);
        let verifying_key = signing_key.verifying_key();
        let public_key_base64 = Base64.encode(verifying_key.to_bytes());

        let base_ws_url = format!("ws://{}", relay_addr);
        let register_url = format!("{}/register", base_ws_url);

        let (control_closed_tx, control_closed_rx) = oneshot::channel();
        let server_task = tokio::spawn(run_test_server_control(
            register_url,
            base_ws_url.clone(),
            server_id,
            secret.clone(),
            public_key_base64.clone(),
            control_closed_tx,
        ));

        let mut attempts = 0;
        loop {
            if state.server_entry(&server_id).await.is_some() {
                break;
            }
            attempts += 1;
            if attempts > 200 {
                panic!("Server did not register with relay");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let client_id = Uuid::new_v4();
        let relay_token = {
            let client_id_str = client_id.to_string();
            create_client_token(
                &signing_key,
                server_id,
                &client_id_str,
                "127.0.0.1",
                "client_id",
                &client_id_str,
            )
        }
        .unwrap();

        let connect_url = format!("{}/connect", base_ws_url);
        let (client_ws, _) = connect_with_protocol(&connect_url).await.unwrap();
        let (mut client_sink, mut client_stream) = client_ws.split();

        let connect_msg = json!({
            "type": "connect",
            "server_id": server_id.to_string(),
            "relay_token": relay_token,
            "client_id": client_id.to_string(),
            "client_version": "integration-test",
        });
        client_sink
            .send(WsMessage::Text(connect_msg.to_string().into()))
            .await
            .unwrap();

        let ack_msg = client_stream.next().await.unwrap().unwrap();
        let ack_text = match ack_msg {
            WsMessage::Text(text) => text,
            other => panic!("Expected text ack, got {:?}", other),
        };
        let ack_value: Value = serde_json::from_str(ack_text.as_str()).unwrap();
        assert_eq!(ack_value["status"], "ok");

        let tunnel_id = ack_value["tunnel_id"]
            .as_str()
            .expect("missing tunnel_id in ack")
            .to_string();

        let ready_msg = json!({
            "type": "tunnel_ready",
            "tunnel_id": tunnel_id,
            "role": "client",
        });
        client_sink
            .send(WsMessage::Text(ready_msg.to_string().into()))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;

        drop(client_sink);
        drop(client_stream);

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            _ = control_closed_rx => {
                panic!("Control connection terminated after abrupt client drop");
            }
        }

        relay_handle.abort();
        let _ = relay_handle.await;
        server_task.abort();
        let _ = server_task.await;
    }

    async fn run_test_server_control(
        register_url: String,
        base_ws_url: String,
        server_id: Uuid,
        relay_secret: String,
        public_key_base64: String,
        control_closed_tx: oneshot::Sender<()>,
    ) -> Result<()> {
        let (ws_stream, _) = connect_with_protocol(&register_url).await?;
        let (mut sink, mut stream) = ws_stream.split();

        let register_msg = json!({
            "type": "register",
            "server_id": server_id.to_string(),
            "relay_secret": relay_secret,
            "server_version": "test",
            "capabilities": ["relay.v1"],
            "public_key": public_key_base64,
            "max_tunnels": 2,
            "tls_authority": "handcontrol.local:50051",
        });
        sink.send(WsMessage::Text(register_msg.to_string().into()))
            .await?;

        let ack_msg = stream
            .next()
            .await
            .ok_or_else(|| anyhow!("Missing register ack"))??;
        let ack_text = match ack_msg {
            WsMessage::Text(text) => text,
            other => anyhow::bail!("Unexpected register ack frame: {:?}", other),
        };
        let ack_value: Value = serde_json::from_str(ack_text.as_str())?;
        if ack_value["status"] != "ok" {
            anyhow::bail!("Register failed: {:?}", ack_value);
        }

        let mut tunnel_tasks = Vec::new();

        while let Some(msg) = stream.next().await {
            match msg {
                Ok(WsMessage::Text(text)) => {
                    if let Ok(value) = serde_json::from_str::<Value>(text.as_str()) {
                        if value["type"] == "open_tunnel" {
                            if let (Some(tunnel_id), Some(server_secret)) =
                                (value["tunnel_id"].as_str(), value["server_secret"].as_str())
                            {
                                let tunnel_url = format!(
                                    "{}/tunnel/{}?role=server&token={}",
                                    base_ws_url,
                                    tunnel_id,
                                    form_urlencoded::byte_serialize(server_secret.as_bytes())
                                        .collect::<String>()
                                );
                                let tunnel_id_string = tunnel_id.to_string();
                                let handle = tokio::spawn(async move {
                                    if let Err(err) =
                                        handle_test_server_tunnel(tunnel_url, tunnel_id_string)
                                            .await
                                    {
                                        tracing::error!("Test tunnel failed: {:?}", err);
                                    }
                                });
                                tunnel_tasks.push(handle);
                            }
                        }
                    }
                }
                Ok(WsMessage::Ping(payload)) => {
                    sink.send(WsMessage::Pong(payload)).await?;
                }
                Ok(WsMessage::Close(_)) => break,
                Ok(_) => {}
                Err(err) => {
                    return Err(anyhow!(err));
                }
            }
        }

        for handle in tunnel_tasks {
            handle.abort();
            let _ = handle.await;
        }

        let _ = control_closed_tx.send(());
        Ok(())
    }

    async fn handle_test_server_tunnel(tunnel_url: String, tunnel_id: String) -> Result<()> {
        let (ws_stream, _) = connect_with_protocol(&tunnel_url).await?;
        let (mut sink, mut stream) = ws_stream.split();

        let ready_msg = json!({
            "type": "tunnel_ready",
            "tunnel_id": tunnel_id,
            "role": "server",
        });
        sink.send(WsMessage::Text(ready_msg.to_string().into()))
            .await?;

        while let Some(msg) = stream.next().await {
            match msg {
                Ok(WsMessage::Binary(_)) => {}
                Ok(WsMessage::Ping(payload)) => {
                    sink.send(WsMessage::Pong(payload)).await?;
                }
                Ok(WsMessage::Close(_)) => break,
                Ok(_) => {}
                Err(err) => return Err(anyhow!(err)),
            }
        }

        Ok(())
    }

    fn create_client_token(
        signing_key: &SigningKey,
        server_id: Uuid,
        subject: &str,
        audience: &str,
        binding_type: &str,
        binding_value: &str,
    ) -> Result<String> {
        use std::time::{SystemTime, UNIX_EPOCH};

        #[derive(Serialize)]
        struct RelayTokenClaims<'a> {
            iss: &'static str,
            sub: String,
            aud: &'a str,
            exp: u64,
            iat: u64,
            server_id: String,
            server_audience: String,
            binding_type: String,
            binding_value: String,
            permissions: Vec<&'static str>,
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system time before unix epoch")?
            .as_secs();
        let exp = now + 3600;

        let claims = RelayTokenClaims {
            iss: "handcontrol-server",
            sub: subject.to_string(),
            aud: audience,
            exp,
            iat: now,
            server_id: server_id.to_string(),
            server_audience: server_id.to_string(),
            binding_type: binding_type.to_string(),
            binding_value: binding_value.to_string(),
            permissions: vec!["connect"],
        };

        let der = signing_key
            .to_pkcs8_der()
            .context("encode signing key to DER")?;
        let encoding_key = EncodingKey::from_ed_der(der.as_bytes());

        let token = jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), &claims, &encoding_key)
            .context("encode token")?;
        Ok(token)
    }
}
