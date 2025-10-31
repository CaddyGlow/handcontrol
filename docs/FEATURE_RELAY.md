# Feature Plan: Cross-Network Relay Support

**Version:** 1.0
**Date:** 2025-10-30
**Status:** Planned
**Priority:** High (Post Multi-IP and IPv6 Enhancement)
**Dependencies:**
- Multi-IP Enrollment Support (MULTI_IP_ENROLLMENT.md)
- IPv6 Enhancement (FEATURE_IPV6_ENHANCEMENT.md)

---

## Executive Summary

This feature enables HandControl to work across different networks (home, mobile, office) by adding relay server infrastructure. The relay acts as a transparent proxy, maintaining end-to-end mTLS security while enabling connectivity when direct peer-to-peer connection fails.

**Key Design Principle:** Direct connection always preferred; relay is fallback only.

---

## Table of Contents

1. [Overview](#overview)
2. [Motivation](#motivation)
3. [Architecture](#architecture)
4. [Implementation Plan](#implementation-plan)
5. [Deployment Models](#deployment-models)
6. [Security](#security)
7. [Performance](#performance)
8. [Testing](#testing)
9. [Migration](#migration)

---

## Overview

### What is a Relay?

A relay server acts as a rendezvous point for HandControl clients and servers that cannot connect directly due to network constraints (NAT, firewalls, different subnets).

```
Android (Mobile Network)  <--mTLS-->  Relay Server  <--mTLS-->  PC (Home Network)
    10.x.x.x (Carrier NAT)              Public IP                192.168.1.100 (Private)
         |                                  |                             |
         +---- Cannot reach directly -------+-- Relay proxies packets ---+
```

### Core Features

1. **Transparent Relay**: End-to-end mTLS preserved (relay sees only encrypted data)
2. **Automatic Fallback**: Try direct first, use relay only if needed
3. **Flexible Deployment**: Self-hosted or cloud VPS
4. **Hybrid Mode**: Server can act as relay for others (optional)
5. **IPv6 Native**: Relay supports IPv4 and IPv6 dual-stack
6. **Firewall-Friendly Transport**: WebSocket tunnel over TLS 1.3 (port 443)
7. **JWT Authentication**: Server-issued tokens, relay validates

---

## Motivation

### Problem Statement

HandControl currently only works on local networks:
- ❌ Cannot connect from mobile network (4G/5G) to home PC
- ❌ Cannot connect from coffee shop WiFi to home PC
- ❌ Cannot connect across different VLANs/subnets
- ❌ Requires VPN for remote access (complex setup)

### Solution: Relay Server

Add optional relay infrastructure that:
- ✅ Enables cross-network connectivity
- ✅ Maintains security (end-to-end mTLS)
- ✅ Provides automatic fallback (try direct first)
- ✅ Supports self-hosting (no cloud dependency)
- ✅ Works with IPv6 networks

### Use Cases

**Primary Use Case (Single User):**
- User wants to control home PC from anywhere
- Direct connection works at home (same WiFi)
- Relay connection works when away (mobile network)
- User doesn't need to think about network topology

**Secondary Use Cases:**
- Corporate environment with multiple VLANs
- Remote office connectivity
- Traveling with laptop (hotel WiFi to home PC)
- IPv6-only mobile network to IPv4-only home network

---

## Architecture

### High-Level System Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                        HandControl Ecosystem                         │
├─────────────────────────────────────────────────────────────────────┤
│                                                                       │
│  ┌──────────────┐                                   ┌─────────────┐  │
│  │   Android    │                                   │  PC Server  │  │
│  │   Client     │                                   │             │  │
│  └──────┬───────┘                                   └──────┬──────┘  │
│         │                                                  │         │
│         │  1. Try Direct Connection                       │         │
│         │     (Multi-IP, IPv4/IPv6)                       │         │
│         ├─────────────────────────────────────────────────┤         │
│         │                                                  │         │
│         │  ✓ Success? --> Use Direct (FAST)              │         │
│         │  ✗ Failed?  --> Try Relay (below)              │         │
│         │                                                  │         │
│         │  2. Fallback to Relay                           │         │
│         │                                                  │         │
│         │           ┌─────────────────────┐               │         │
│         │           │   Relay Server      │               │         │
│         │           │   (Public IP)       │               │         │
│         │           │   :443 (wss)        │               │         │
│         │           └──────────┬──────────┘               │         │
│         │                      │                           │         │
│         ├──────── TLS 1.3 WebSocket Tunnel ───────────────┤         │
│         │                      │                           │         │
│    [mTLS Handshake + gRPC Data]──>[Binary Relay]──>[mTLS Handshake + gRPC Data] │
│         │                      │                           │         │
│         │  Relay cannot        │  Relay cannot             │         │
│         │  decrypt (no keys)   │  decrypt (no keys)        │         │
│         │                                                  │         │
│         └──────────────────End-to-End mTLS────────────────┘         │
│                                                                       │
└─────────────────────────────────────────────────────────────────────┘
```

### Component Responsibilities

#### 1. Relay Server (New Component)

**Purpose:** Forward encrypted packets between client and server.

**Responsibilities:**
- Accept connections from servers (registration)
- Accept connections from clients (connection requests)
- Validate JWT tokens
- Forward packets bidirectionally
- Track connection state
- Rate limiting and abuse prevention

**Does NOT:**
- Decrypt mTLS traffic (no keys)
- Store sensitive data
- Perform authentication (only validates tokens)
- Modify packet contents

#### 2. PC Server (Updated)

**New Capabilities:**
- Connect to relay server on startup (optional)
- Register server ID with relay
- Generate relay tokens for enrolled clients
- Include relay info in QR codes (optional)
- Auto-reconnect to relay on disconnect

**Configuration:**
```toml
[relay]
enabled = false
relay_server_url = "https://relay.example.com"
relay_auth_secret = "base64-encoded-secret"
max_relay_tunnels = 10
auto_connect = true
include_in_enrollment = true
# allow_self_signed_tls = true               # Optional: accept self-signed relay certificate
# pinned_cert_sha256 = "AA...FF"             # Optional: SHA-256 fingerprint when using self-signed TLS
```

When connecting to a relay with a self-signed certificate, set `allow_self_signed_tls = true`.
For additional protection, supply the relay's SHA-256 certificate fingerprint via `pinned_cert_sha256`
so that only the expected certificate is trusted.

#### 3. Android Client (Updated)

**New Capabilities:**
- Parse relay info from QR codes
- Store relay credentials in database
- Implement relay connection strategy
- Automatic fallback (direct -> relay)
- Connection mode tracking
- UI indicator for connection type

### Relay Tunnel Design

#### Connection Sequence

```
Server Registration Flow:
1. Server opens WebSocket: wss://relay.example.com/register (TLS 1.3, port 443)
2. Server sends Register message (JSON) containing server_id, relay_secret, server_version, public_key
3. Relay validates relay_secret, stores server metadata + public key, responds with RegisterAck
4. WebSocket stays open as a control channel (keep-alive via WebSocket ping/pong)

Client Connection Flow:
1. Client tries direct IPs (multi-IP, IPv6 first)
2. If all direct attempts fail and relay info exists:
   a. Client opens WebSocket: wss://relay.example.com/connect
   b. Client sends Connect message (JSON) with server_id, relay_token
   c. Relay validates relay_token using cached server public key
   d. Relay allocates tunnel_id and returns `ConnectAck` to the client
   e. Relay notifies the registered server over the control channel (`open_tunnel`)
   f. Server opens WebSocket: wss://relay.example.com/tunnel/<tunnel_id>?role=server and sends `tunnel_ready`
   g. Client sends `tunnel_ready` on the original /connect socket
3. Relay switches both WebSockets into binary forwarding mode; all frames after TunnelReady are raw bytes from the original mTLS session between client and server
4. End-to-end mTLS is preserved because the TLS handshake and encrypted HTTP/2 traffic flow untouched through the tunnel
```

#### WebSocket Message Types

All control messages are UTF-8 JSON frames with `type` as the discriminator. The WebSocket handshake uses the `Sec-WebSocket-Protocol: handcontrol-relay.v1` subprotocol.

```json
// Sent by server immediately after /register upgrade
{
  "type": "register",
  "server_id": "<uuid>",
  "relay_secret": "<base64>",
  "server_version": "1.8.0",
  "capabilities": ["relay.v1"],
  "public_key": "<base64-ed25519-public-key>",
  "max_tunnels": 10
}

// Relay acknowledgement
{
  "type": "register_ack",
  "status": "ok",
  "retry_after_seconds": 0
}

// Sent by client after /connect upgrade
{
  "type": "connect",
  "server_id": "<uuid>",
  "relay_token": "<jwt>",
  "client_id": "<uuid>",
  "client_version": "android-1.12.0"
}

// Relay acknowledgement to client
{
  "type": "connect_ack",
  "status": "ok",
  "tunnel_id": "<uuid>",
  "relay_host": "relay.example.com",
  "server_authority": "handcontrol.local:50051",
  "expires_at": 1735689600
}

// Relay instructs registered server to open a tunnel
{
  "type": "open_tunnel",
  "tunnel_id": "<uuid>",
  "client_id": "<uuid>",
  "preferred_protocol": "binary",
  "expires_at": 1735689600,
  "server_secret": "<base64 token>"
}

// Server acknowledges readiness and switches to binary mode
{
  "type": "tunnel_ready",
  "tunnel_id": "<uuid>",
  "role": "server"
}

// Client mirrors readiness on the same socket
{
  "type": "tunnel_ready",
  "tunnel_id": "<uuid>",
  "role": "client"
}
```

Clients wait for `connect_ack` before emitting `tunnel_ready`. After both sides send `tunnel_ready`, the relay no longer emits JSON frames on that WebSocket. Binary frames are forwarded byte-for-byte between client and server. Each tunnel WebSocket carries exactly one bidirectional TLS session, so no additional multiplexing headers are required.

#### Keep-alives and Error Handling

- Control WebSockets rely on the native WebSocket ping/pong to detect dead connections (interval: 30 seconds, timeout: 15 seconds).
- Tunnel WebSockets use application-level heartbeat frames (single-byte 0x00 payload every 20 seconds) while idle to keep NAT bindings alive.
- If the relay cannot validate a token or secret it sends an error frame and closes the WebSocket with code `4401` (custom “unauthorized”).
- Idle tunnels are closed after `idle_timeout_seconds` (default 600) with clean close code `1000`.

#### Synchronization & Race Prevention

- The relay tracks each tunnel in a `Pending` state until it has received `tunnel_ready` from both server and client; only then does it transition to `Active` and begin forwarding bytes.
- If the client sends `tunnel_ready` before the server connects, the relay queues the signal and starts a 5-second timer; if the server does not arrive before timeout, the relay responds with an error and closes the socket gracefully.
- Conversely, if the server opens the tunnel first, its `tunnel_ready` is buffered until the client-initialized socket confirms readiness.
- This handshake ensures neither side processes application data until the opposite endpoint is confirmed, eliminating race conditions where one party might start writing TLS records into a half-open tunnel.

#### Preserving End-to-End TLS

Because the relay only sees opaque TLS bytes inside the WebSocket tunnel, it never terminates or inspects the mutual TLS session between Android and the PC server. Certificate validation and client authentication continue to happen exactly as they do on a direct connection. The relay simply copies binary frames in both directions.

### JWT Token Format

**Token Structure:**
```json
{
  "header": {
    "alg": "EdDSA",           // Ed25519 signature
    "typ": "JWT"
  },
  "payload": {
    "iss": "handcontrol-server",
    "sub": "<client_id>",
    "aud": "relay.example.com",
    "exp": 1735689600,        // Expiry (Unix timestamp)
    "iat": 1735603200,        // Issued at
    "server_id": "<server_uuid>",
    "permissions": ["connect"]
  },
  "signature": "..."
}
```

**Token Lifecycle:**
1. Server generates Ed25519 keypair on first run
2. Server stores private key securely
3. Server shares public key with relay (out-of-band or during registration)
4. During client enrollment, server generates JWT token
5. Token included in enrollment response (QR or approval mode)
6. Android stores token in database
7. Android presents token when connecting via relay
8. Relay validates signature using server's public key
9. Relay checks expiry and claims
10. Relay allows/denies connection based on validation

---

## Implementation Plan

### Rust Dependency Updates

Before coding, update the three workspace manifests to add the WebSocket and HTTP tooling this design requires:

- `Cargo.toml` (workspace root): add shared dependencies for `axum = "0.7"`, `tokio-tungstenite = "0.25"`, `tungstenite = "0.21"`, `futures-util = "0.3"`, and `tower = "0.5"`.
- `relay/Cargo.toml`: depend on `axum`, `hyper = "1"`, `http = "1"`, `tokio-tungstenite`, `tungstenite`, and `futures-util`, dropping the now-unused `tonic`/`prost` crates after the WebSocket migration.
- `server/Cargo.toml`: add `tokio-tungstenite`, `tungstenite`, `futures-util`, and `url` to support the client-side tunnel connector.

Validate `cargo metadata` still succeeds after these edits before proceeding with code changes.

#### 1.0 Relay Configuration

Ship a default `relay/relay.toml` alongside the binary so administrators have a starting point:

```toml
bind_address = "0.0.0.0"
port = 443
handshake_timeout_seconds = 5

[registration_secrets]
"00000000-0000-0000-0000-000000000001" = "base64-secret-generated-per-server"
```

The CLI accepts an optional `--config` argument; when omitted, it falls back to `relay.toml` in the working directory.

### Phase 1: Relay Server Foundation (Weeks 1-2)

#### 1.1 Project Structure

Create new crate in workspace:

```
handcontrol/
├── Cargo.toml (workspace root)
├── server/
│   └── Cargo.toml (handcontrol-server)
├── relay/                       # NEW
│   ├── Cargo.toml (handcontrol-relay)
│   └── src/
│       ├── main.rs
│       ├── tunnel.rs
│       ├── auth.rs
│       └── config.rs
└── proto/ (shared)
```

#### 1.2 Relay Server Core

**File:** `relay/src/main.rs`

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt::init();

    let config_path = cli.config.unwrap_or_else(default_config_path);
    let config = load_config(&config_path)?;

    let state = Arc::new(AppState::new(config));

    let app = Router::new()
        .route("/register", get(register_handler))
        .route("/connect", get(connect_handler))
        .route("/tunnel/:tunnel_id", get(tunnel_handler))
        .with_state(state.clone());

    let addr: SocketAddr = state.listen_addr.parse()?;
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app.into_make_service()).await?;
    Ok(())
}

struct RegisteredServer {
    control_tx: mpsc::Sender<ServerCommand>,
    decoding_key: Arc<DecodingKey>,
}

impl ServerCommand {
    fn into_message(self) -> Message {
        match self {
            ServerCommand::OpenTunnel { tunnel_id, client_id, preferred_protocol, expires_at } => {
                Message::Text(
                    serde_json::json!({
                        "type": "open_tunnel",
                        "tunnel_id": tunnel_id.to_string(),
                        "client_id": client_id.to_string(),
                        "preferred_protocol": preferred_protocol,
                        "expires_at": expires_at,
                    })
                    .to_string(),
                )
            }
        }
    }
}

async fn handle_register_socket(mut socket: WebSocket, state: Arc<AppState>) -> Result<()> {
    let payload = recv_text(&mut socket, "register")?;
    let msg: RegisterPayload = serde_json::from_str(&payload)?;
    let server_id = Uuid::parse_str(&msg.server_id)?;

    if !state.validate_secret(&server_id, &msg.relay_secret).await {
        send_error(&mut socket, "register_ack", "unauthorized").await?;
        return Ok(());
    }

    let raw_key = Base64.decode(msg.public_key.as_bytes())?;
    let verifying_key = VerifyingKey::from_bytes(raw_key.as_slice().try_into()?)?;
    let public_key_der = verifying_key.to_public_key_der()?;
    let decoding_key = Arc::new(DecodingKey::from_ed_der(public_key_der.as_ref()));

    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "register_ack",
                "status": "ok",
                "retry_after_seconds": 0
            })
            .to_string(),
        ))
        .await?;

    let (tx, mut rx) = mpsc::channel(32);
    state.upsert_server(
        server_id,
        Arc::new(RegisteredServer {
            control_tx: tx,
            decoding_key,
        }),
    ).await;

    while let Some(cmd) = rx.recv().await {
        socket.send(cmd.into_message()).await?;
    }

    state.remove_server(&server_id).await;
    Ok(())
}

async fn handle_connect_socket(mut socket: WebSocket, state: Arc<AppState>) -> Result<()> {
    let payload = recv_text(&mut socket, "connect")?;
    let msg: ConnectPayload = serde_json::from_str(&payload)?;
    let server_id = Uuid::parse_str(&msg.server_id)?;
    let client_id = Uuid::parse_str(&msg.client_id)?;

    let Some(server_entry) = state.server_entry(&server_id).await else {
        send_error(&mut socket, "connect_ack", "server_not_registered").await?;
        return Ok(());
    };

    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_audience(&[state.relay_host.as_str()]);
    let claims = match decode::<RelayClaims>(&msg.relay_token, server_entry.decoding_key.as_ref(), &validation) {
        Ok(data) => data.claims,
        Err(_) => {
            send_error(&mut socket, "connect_ack", "invalid_token").await?;
            return Ok(());
        }
    };

    let claims_server = match Uuid::parse_str(&claims.server_id) {
        Ok(uuid) => uuid,
        Err(_) => {
            send_error(&mut socket, "connect_ack", "invalid_token").await?;
            return Ok(());
        }
    };

    if claims_server != server_id {
        send_error(&mut socket, "connect_ack", "server_mismatch").await?;
        return Ok(());
    }

    if claims.sub != client_id.to_string() {
        send_error(&mut socket, "connect_ack", "client_mismatch").await?;
        return Ok(());
    }

    if !claims.permissions.iter().any(|p| p == "connect") {
        send_error(&mut socket, "connect_ack", "permission_denied").await?;
        return Ok(());
    }

    let tunnel_id = Uuid::new_v4();
    let tunnel_entry = state.create_tunnel(tunnel_id, Instant::now()).await;

    let expires_at = SystemTime::now()
        .checked_add(state.handshake_timeout)
        .and_then(|deadline| deadline.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|dur| dur.as_secs())
        .unwrap_or(0);

    let server_secret = generate_server_secret();

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
        send_error(&mut socket, "connect_ack", "server_unreachable").await?;
        state.remove_tunnel(&tunnel_id).await;
        return Ok(());
    }

    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "connect_ack",
                "status": "ok",
                "tunnel_id": tunnel_id.to_string(),
                "relay_host": state.relay_host,
                "server_authority": server_entry.tls_authority,
                "expires_at": expires_at,
            })
            .to_string(),
        ))
        .await?;

    // ... await tunnel_ready and transition to forwarding ...
    Ok(())
}
```

#### 1.3 Authentication & JWT Handling

- Registration now expects the server to send a base64 Ed25519 public key. The relay converts it to DER, caches a `DecodingKey`, and associates a control channel with the server ID.
- During `/connect`, the relay validates JWTs with `decode::<RelayClaims>` and emits explicit error codes (`invalid_token`, `server_not_registered`, `permission_denied`, `server_unreachable`). The `connect_ack` response carries a server-generated tunnel id plus an expiration deadline.
- Tunnel state is tracked via `TunnelHandle`, ensuring both client and server deliver `tunnel_ready` before frames are forwarded. Additional cleanup (removing tunnel state, notifying clients) is handled after completion.

#### 1.4 Configuration

**File:** `relay/config.toml`

```toml
[relay]
bind_address = "::"
port = 443
public_hostname = "relay.example.com"
tls_cert_path = "/etc/handcontrol-relay/cert.pem"
tls_key_path = "/etc/handcontrol-relay/key.pem"
websocket_subprotocol = "handcontrol-relay.v1"

[auth]
require_tokens = true
# Public keys are registered dynamically by servers
# or loaded from directory
public_keys_dir = "/etc/handcontrol-relay/server-keys"

[limits]
max_registered_servers = 100
max_tunnels_per_server = 10
max_bandwidth_mbps = 50
connection_timeout_seconds = 300
idle_timeout_seconds = 600

[logging]
level = "info"
format = "json"
```

**Deliverable:** Working relay server that accepts connections and validates tokens.

---

### Phase 2: Server Relay Integration (Weeks 3-4)

#### 2.1 Server Configuration

**File:** `server/src/config/mod.rs`

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RelayConfig {
    #[serde(default)]
    pub enabled: bool,

    pub relay_server_url: Option<String>,
    pub relay_auth_secret: Option<String>,
    pub max_relay_tunnels: Option<u32>,

    #[serde(default = "default_true")]
    pub auto_connect: bool,

    #[serde(default = "default_true")]
    pub include_in_enrollment: bool,

    #[serde(default = "default_relay_reconnect_delay")]
    pub reconnect_delay_seconds: u64,
}

fn default_relay_reconnect_delay() -> u64 { 30 }
```

#### 2.2 Relay Client Module

**File:** `server/src/relay/client.rs` (NEW)

Uses `tokio_tungstenite` for WebSocket transport and bridges binary frames back to the local gRPC listener without terminating TLS. The client caches multiple tunnels and reacts to `open_tunnel` commands sent over the control channel by the relay.

```rust
pub struct RelayClient {
    config: RelayConfig,
    server_id: Uuid,
    token_issuer: Arc<TokenIssuer>,
    local_grpc_endpoint: SocketAddr,
    control: Option<ControlChannel>,
    state: Arc<RwLock<RelayClientState>>,
}

enum RelayClientState {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

struct ControlChannel {
    sink: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
}

impl RelayClient {
    pub async fn connect(&mut self) -> Result<()> {
        let url = format!("{}/register", self.config.relay_server_url()?);
        let (ws, _) = connect_async(&url).await?;
        let (mut sink, mut stream) = ws.split();

        let register = RegisterMessage {
            server_id: self.server_id,
            relay_secret: self.config.relay_auth_secret.clone()?,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            capabilities: vec!["relay.v1".into()],
            public_key: self.token_issuer.public_key_base64(),
            max_tunnels: self.config.max_relay_tunnels.unwrap_or(10),
        };
        sink.send(Message::text(serde_json::to_string(&register)?))
            .await?;

        match stream.next().await {
            Some(Ok(Message::Text(payload))) => {
                let ack: RegisterAck = serde_json::from_str(&payload)?;
                if ack.status != "ok" {
                    return Err(anyhow!("Relay registration failed: {}", ack.status));
                }
            }
            other => return Err(anyhow!("Unexpected register response: {other:?}")),
        }

        self.control = Some(ControlChannel { sink, stream });
        *self.state.write().await = RelayClientState::Connected;

        tokio::spawn(self.listen_for_control_messages());
        Ok(())
    }

    async fn listen_for_control_messages(&mut self) {
        let Some(control) = &mut self.control else { return };
        while let Some(Ok(Message::Text(payload))) = control.stream.next().await {
            match serde_json::from_str::<ControlEnvelope>(&payload) {
                Ok(ControlEnvelope::OpenTunnel { tunnel_id, client_id, .. }) => {
                    tracing::info!("Relay requested tunnel {tunnel_id} for client {client_id}");
                    if let Err(err) = self.spawn_tunnel_task(tunnel_id).await {
                        tracing::error!("Failed to open relay tunnel: {err:?}");
                    }
                }
                Ok(ControlEnvelope::Ping { .. }) => {
                    let _ = control.sink.send(Message::Text(r#"{"type":"pong"}"#.into())).await;
                }
                Err(err) => tracing::warn!("Invalid control message: {err:?}"),
            }
        }
    }

    async fn spawn_tunnel_task(&self, tunnel_id: Uuid) -> Result<()> {
        let url = format!("{}/tunnel/{}?role=server", self.config.relay_server_url()?, tunnel_id);
        let (ws, _) = connect_async(&url).await?;
        let (mut sink, mut stream) = ws.split();

        sink.send(Message::text(json!({
            "type": "tunnel_ready",
            "tunnel_id": tunnel_id
        }).to_string()))
        .await?;

        // Bridge WebSocket frames to the existing gRPC listener; TLS terminates at the local server.
        pump_tls_between_websocket(stream, sink, self.local_grpc_endpoint.clone()).await
    }
}
```

#### 2.3 Token Generation

**File:** `server/src/relay/tokens.rs` (NEW)

```rust
use ed25519_dalek::{
    pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
    SigningKey, VerifyingKey,
};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

pub struct TokenIssuer {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
    server_id: Uuid,
}

impl TokenIssuer {
    pub fn new(server_id: Uuid) -> Self {
        // Load or generate Ed25519 keypair
        let signing_key = load_or_generate_key()?;
        let verifying_key = VerifyingKey::from(&signing_key);

        Self {
            signing_key,
            verifying_key,
            server_id,
        }
    }

    pub fn generate_relay_token(
        &self,
        client_id: &str,
        relay_url: &str,
        ttl_hours: u64,
    ) -> Result<String> {
        let claims = Claims {
            iss: "handcontrol-server".to_string(),
            sub: client_id.to_string(),
            aud: relay_url.to_string(),
            exp: (Utc::now() + Duration::hours(ttl_hours)).timestamp() as u64,
            iat: Utc::now().timestamp() as u64,
            server_id: self.server_id.to_string(),
            permissions: vec!["connect".to_string()],
        };

        let token = encode(
            &Header::new(Algorithm::EdDSA),
            &claims,
            &EncodingKey::from_ed25519_pem(
                self.signing_key
                    .to_pkcs8_pem(LineEnding::LF)?
                    .as_bytes(),
            )?,
        )?;

        Ok(token)
    }

    pub fn public_key_pem(&self) -> String {
        self.verifying_key
            .to_public_key_pem(LineEnding::LF)
            .expect("verifying key to pem")
    }

    pub fn public_key_base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(self.verifying_key.to_bytes())
    }
}
```

#### 2.4 Update Enrollment Responses

**File:** `server/src/grpc/server.rs`

```rust
async fn enroll(
    &self,
    request: Request<EnrollRequest>,
) -> Result<Response<EnrollResponse>, Status> {
    // ... existing enrollment logic ...

    // Generate relay token if relay enabled
    let relay_token = if self.config.relay.enabled {
        let token_issuer = self.token_issuer.lock().await;
        Some(token_issuer.generate_relay_token(
            &client_id,
            self.config.relay.relay_server_url.as_ref().unwrap(),
            24, // 24 hour TTL
        )?)
    } else {
        None
    };

    Ok(Response::new(EnrollResponse {
        success: true,
        client_id,
        relay_info: relay_token.map(|token| RelayInfo {
            relay_url: self.config.relay.relay_server_url.clone().unwrap(),
            relay_token: token,
            relay_required: false,
        }),
        ..Default::default()
    }))
}
```

**Deliverable:** Server can connect to relay and generate tokens for clients.

---

### Phase 3: Android Relay Support (Weeks 5-6)

#### 3.1 Database Schema Updates

Already prepared in MULTI_IP_ENROLLMENT.md:

```kotlin
@Entity(tableName = "enrolled_servers")
data class EnrolledServer(
    // ... existing fields ...
    val relayEnabled: Boolean = false,
    val relayUrl: String? = null,
    val relayToken: String? = null,
    val lastConnectionMode: ConnectionMode = ConnectionMode.DIRECT,
)
```

#### 3.2 Relay Connection Strategy

**File:** `app/src/main/kotlin/com/handcontrol/core/network/RelayConnectionStrategy.kt` (NEW)

```kotlin
class RelayConnectionStrategy(
    private val channelFactory: MtlsGrpcChannelFactory,
    private val tunnelFactory: RelayTunnelFactory,
) : ConnectionStrategy {

    override suspend fun connect(attempt: ConnectionAttempt): Result<ManagedChannel> {
        return when (attempt) {
            is ConnectionAttempt.Relay -> connectViaRelay(attempt)
            else -> Result.failure(UnsupportedOperationException())
        }
    }

    private suspend fun connectViaRelay(attempt: ConnectionAttempt.Relay): Result<ManagedChannel> = withContext(Dispatchers.IO) {
        runCatching {
            val tunnel = tunnelFactory.openTunnel(
                RelayTunnelRequest(
                    relayUrl = attempt.relayUrl,
                    serverId = attempt.serverId,
                    relayToken = attempt.relayToken,
                    tlsConfig = attempt.tlsConfig,
                )
            )

            val channel = channelFactory.createChannelOverTunnel(
                authority = attempt.authority,
                tunnel = tunnel,
            )

            channel
        }.onFailure { Timber.e(it, "Relay connection failed") }
    }
}
```

#### 3.3 Relay Tunnel Factory

**File:** `app/src/main/kotlin/com/handcontrol/core/network/RelayTunnelFactory.kt` (NEW)

```kotlin
class RelayTunnelFactory(
    private val okHttpClient: OkHttpClient,
    private val json: Json,
    private val dispatcher: CoroutineDispatcher = Dispatchers.IO,
) {

    suspend fun openTunnel(request: RelayTunnelRequest): RelayTunnel = withContext(dispatcher) {
        val connectUrl = request.relayUrl.toHttpUrl().newBuilder()
            .addPathSegment("connect")
            .build()

        val listener = RelayWebSocketListener(json)
        val webSocket = okHttpClient.newWebSocket(
            Request.Builder()
                .url(connectUrl)
                .header("Sec-WebSocket-Protocol", "handcontrol-relay.v1")
                .build(),
            listener,
        )

        val connectMessage = ConnectMessage(
            serverId = request.serverId,
            relayToken = request.relayToken,
            clientId = request.clientId,
            clientVersion = BuildConfig.VERSION_NAME,
        )
        webSocket.send(json.encodeToString(connectMessage))

        val tunnelInfo = listener.awaitTunnelReady()
        RelayTunnel(
            websocket = webSocket,
            output = listener.binarySink,
            input = listener.binarySource,
            tunnelId = tunnelInfo.tunnelId,
            authority = request.authority,
        )
    }
}
```

#### 3.4 UI Updates

**File:** `app/src/main/kotlin/com/handcontrol/feature/servers/ServerCard.kt`

```kotlin
@Composable
fun ConnectionStatusIndicator(
    connectionMode: ConnectionMode,
    isConnected: Boolean
) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Icon(
            imageVector = when {
                !isConnected -> Icons.Default.CloudOff
                connectionMode == ConnectionMode.DIRECT -> Icons.Default.Wifi
                connectionMode == ConnectionMode.RELAY -> Icons.Default.Cloud
                else -> Icons.Default.Help
            },
            contentDescription = null,
            tint = when {
                !isConnected -> MaterialTheme.colorScheme.error
                connectionMode == ConnectionMode.DIRECT -> MaterialTheme.colorScheme.primary
                else -> MaterialTheme.colorScheme.secondary
            }
        )
        Spacer(modifier = Modifier.width(4.dp))
        Text(
            text = when {
                !isConnected -> "Offline"
                connectionMode == ConnectionMode.DIRECT -> "Direct"
                connectionMode == ConnectionMode.RELAY -> "Relay"
                else -> "Unknown"
            },
            style = MaterialTheme.typography.bodySmall
        )
    }
}
```

**Deliverable:** Android can connect via relay and display connection mode.

---

### Phase 4: IPv6 Relay Support (Week 7)

#### 4.1 Dual-Stack Relay Binding

Relay server binds to `::` by default (dual-stack):

```rust
let addr = "[::]:443".parse()?;
let socket = bind_tcp_listener(addr)?;
```

#### 4.2 IPv6 Relay URLs

Support IPv6 literal addresses in relay URLs:

```
https://[2001:db8::1]
https://relay.example.com        (resolves to IPv4 or IPv6)
```

#### 4.3 Connection Preference

When connecting to relay:
1. Try IPv6 first if available
2. Fallback to IPv4
3. Use Happy Eyeballs if relay hostname resolves to multiple IPs

**Deliverable:** Relay works seamlessly with IPv6 networks.

---

### Phase 5: Testing & Documentation (Weeks 8-9)

#### 5.1 Integration Tests

**Relay Server Tests:**
```rust
#[tokio::test]
async fn test_server_registration() {
    // Start relay server
    // Connect mock server
    // Verify registration succeeds
}

#[tokio::test]
async fn test_client_connection_through_relay() {
    // Register mock server
    // Connect mock client
    // Send data through tunnel
    // Verify end-to-end delivery
}

#[tokio::test]
async fn test_token_validation() {
    // Test valid token
    // Test expired token
    // Test invalid signature
    // Test missing claims
}
```

> **Tip:** End-to-end relay scenarios are included in the standard test suite. Run `cargo test -p handcontrol-relay` to exercise the register → connect → tunnel happy path and timeout failure case locally.

**Android Tests:**
```kotlin
@Test
fun testRelayFallback() = runTest {
    // Mock direct connection failure
    // Mock relay connection success
    // Verify relay used automatically
}

@Test
fun testConnectionModePersistence() {
    // Connect via relay
    // Verify lastConnectionMode = RELAY
    // Reconnect
    // Verify relay preferred
}
```

#### 5.2 Load Testing

```bash
# Simulate 100 concurrent tunnels
./load-test-relay.sh --tunnels 100 --duration 300s

# Measure:
# - Latency overhead (relay vs direct)
# - Bandwidth throughput
# - Memory usage
# - CPU usage
# - Connection establishment time
```

#### 5.3 Documentation

- **User Guide**: Setting up relay server (cloud/self-hosted)
- **Admin Guide**: Relay server deployment and configuration
- **Security Guide**: Token management and key rotation
- **Troubleshooting Guide**: Common relay connection issues
- **API Reference**: Relay protocol documentation

**Deliverable:** Production-ready relay with comprehensive docs.

---

## Deployment Models

### Model 1: Cloud VPS (Recommended for Single User)

**Setup:**
```bash
# On DigitalOcean/Linode/AWS VPS
curl -sSL https://get.handcontrol.dev/relay | bash

# Or manual:
docker run -d \
  -p 443:443 \
  -v /etc/handcontrol-relay:/config \
  handcontrol/relay:latest
```

**Pros:**
- Simple setup
- Always available
- Good performance (< 50ms latency typically)
- Low cost ($5-10/month)

**Cons:**
- Requires VPS subscription
- Single point of failure

### Model 2: Self-Hosted (Home Server)

**Setup:**
```bash
# On home server with port forwarding
systemctl enable --now handcontrol-relay
```

**Pros:**
- No recurring cost
- Full control
- Can run on existing hardware

**Cons:**
- Requires port forwarding (443)
- Residential IP may be blocked by some networks
- Depends on home internet uptime

### Model 3: Integrated Mode (Server as Relay)

**Setup:**
```toml
[relay.server_mode]
enabled = true
listen_port = 50053
max_clients = 10
```

**Pros:**
- No separate infrastructure
- Works immediately
- Good for testing

**Cons:**
- Server must be publicly reachable
- Limited scalability
- Not suitable for production

---

## Security

### Threat Model

#### Threats Mitigated

1. **Relay Cannot Decrypt Traffic**
   - End-to-end mTLS preserved
   - Relay only sees encrypted bytes forwarded inside WebSocket frames
   - No certificate trust delegation

2. **Unauthorized Relay Access**
   - JWT tokens required
   - Server-issued, relay-validated
   - Short expiry (24 hours default)

3. **Token Theft**
   - Tokens stored securely in Android Keystore
   - Limited scope (single server_id)
   - Revocable by server

4. **Relay Impersonation**
   - Relay URL included in JWT `aud` claim
   - Client validates relay certificate
   - MITM impossible with proper TLS

#### Threats NOT Mitigated

1. **Relay Availability**
   - If relay down, cross-network connection fails
   - Mitigation: Use reliable hosting, monitoring

2. **Relay Performance**
   - Relay adds latency (typically 20-50ms)
   - Mitigation: Deploy relay geographically close

3. **Traffic Analysis**
   - Relay can see traffic patterns (not contents)
   - Mitigation: Acceptable for use case

### Key Management

#### Server Keys (Ed25519)

**Generation:**
```bash
openssl genpkey -algorithm Ed25519 -out server-relay-key.pem
openssl pkey -in server-relay-key.pem -pubout -out server-relay-pubkey.pem
```

**Storage:**
- Private key: `~/.config/handcontrol/relay-key.pem` (600 permissions)
- Public key: Automatically delivered to relay during `/register` handshake; mirrored on disk for auditing

**Rotation:**
1. Generate new keypair
2. Update server config to point at new private key
3. Reconnect `/register` control WebSocket so the relay stores the new `public_key`
4. Issue new tokens to clients (gradual rollout)
5. Decommission old key after all clients updated

#### Relay Secrets

**Server Registration Secret:**
- Random 32-byte value
- Base64-encoded
- Shared between server and relay (out-of-band)
- Used only for initial registration

**Distribution Flow:**
1. Administrator runs `handcontrol relay secret generate` on the relay host; command writes `/etc/handcontrol-relay/registration-secrets/<server_id>.b64`.
2. The relay CLI prints a one-time copyable secret that is securely transferred to the target PC server (SSH, password manager entry, or QR during enrollment).
3. Admin pastes the secret into `handcontrol.toml` on the PC server (`relay_auth_secret`), or for fresh setups embeds it in the server enrollment QR.
4. On first `/register`, the server presents the secret. The relay marks it as “consumed” and rotates it to an internal HMAC key; subsequent reconnects use a signed timestamp challenge instead of the raw secret.
5. For revocation, the relay admin deletes the secret file and issues a new one—clients must update the server config before reconnecting.

**Generation:**
```bash
openssl rand -base64 32
```

---

## Performance

### Latency Impact

**Measurements (typical):**
- Direct connection: 5-20ms
- Relay connection: 25-70ms
- Added latency: 20-50ms

**Optimization:**
- Deploy relay geographically close to users
- Use relay with good network connectivity
- Consider multiple relay regions for global use

### Bandwidth

**Relay Overhead:**
- Protocol framing: ~5% overhead
- Keep-alive pings: ~100 bytes/second
- Total overhead: negligible for typical use

**Limits:**
- Configurable per-server bandwidth limits
- Default: 50 Mbps per tunnel
- Sufficient for command execution (< 1 Mbps typically)

### Resource Usage

**Relay Server (100 tunnels):**
- CPU: < 5% (4-core VPS)
- Memory: ~500 MB
- Network: Depends on client usage
- Disk: < 100 MB (logs)

**Recommended VPS:**
- 2 vCPU
- 2 GB RAM
- 50 GB SSD
- Cost: $10-15/month

---

## Testing

### Test Matrix

| Scenario | Direct | Relay | Expected |
|----------|--------|-------|----------|
| Same WiFi | ✓ | - | Direct used |
| Mobile to Home | ✗ | ✓ | Relay fallback |
| IPv6-only client | ✓/✗ | ✓ | Depends on server IPv6 |
| Relay down | ✗ | ✗ | Clear error message |
| Token expired | - | ✗ | Graceful failure |
| Direct fails mid-session | ✓ | ✓ | Auto-reconnect via relay |

### Manual Test Plan

1. **Basic Relay Flow**
   - Enroll with relay enabled
   - Verify relay info in QR code
   - Block direct connection
   - Verify relay connection succeeds
   - Confirm WebSocket negotiates `Sec-WebSocket-Protocol: handcontrol-relay.v1`
   - Execute command via relay

2. **Token Management**
   - Enroll and get token
   - Verify token stored
   - Modify token (invalidate)
   - Verify connection fails
   - Re-enroll and get new token

3. **Failover**
   - Connect via direct
   - Disconnect network (simulate)
   - Verify fallback to relay
   - Restore network
   - Verify preference for direct on next connection

4. **Performance**
   - Measure direct latency (ping)
   - Measure relay latency (ping)
   - Compare command execution time
   - Verify acceptable degradation

---

## Migration

### Rollout Strategy

**Phase 1: Server Update (No Client Impact)**
1. Update server to version with relay support
2. Do NOT enable relay yet (`relay.enabled = false`)
3. Verify server still works normally
4. Test token generation (hidden feature)

**Phase 2: Relay Server Deployment**
1. Deploy relay server on VPS
2. Configure server to connect to relay
3. Enable relay (`relay.enabled = true`)
4. Verify server registers successfully
5. Existing clients unaffected (no relay support yet)

**Phase 3: Android Update (Gradual Rollout)**
1. Release Android app with relay support
2. Users update gradually
3. Old app version still works (ignores relay info)
4. New app version uses relay automatically when needed
5. Monitor metrics (direct vs relay usage)

**Phase 4: Optimization**
1. Analyze connection patterns
2. Adjust token TTL based on usage
3. Fine-tune relay configuration
4. Deploy additional relay regions if needed

### Backward Compatibility

**Guaranteed:**
- Old Android app works with new server (ignores relay info)
- New Android app works with old server (no relay support)
- Direct connections always work (relay is additive)
- Existing enrollments remain valid

**Not Compatible:**
- Old Android app cannot use relay (expected, requires update)

---

## Success Criteria

- [ ] Relay server accepts and routes connections
- [ ] Server can register with relay automatically
- [ ] Server generates valid JWT tokens
- [ ] Android can connect via relay when direct fails
- [ ] End-to-end mTLS preserved through relay
- [ ] Connection latency < 100ms via relay (95th percentile)
- [ ] Relay handles 100 concurrent tunnels
- [ ] Token validation works correctly
- [ ] Automatic fallback (direct -> relay) works
- [ ] Connection mode tracked and persisted
- [ ] UI shows connection type (direct/relay)
- [ ] Documentation complete (user + admin guides)
- [ ] Integration tests pass
- [ ] Load tests pass
- [ ] Security audit complete

---

## Timeline

**Total: 9 weeks (45 working days)**

| Phase | Duration | Focus |
|-------|----------|-------|
| 1. Relay Server Foundation | 2 weeks | Core relay server, auth, config |
| 2. Server Integration | 2 weeks | Connect to relay, token generation |
| 3. Android Integration | 2 weeks | Relay connection strategy, UI |
| 4. IPv6 Support | 1 week | Dual-stack relay, IPv6 testing |
| 5. Testing & Docs | 2 weeks | Integration tests, load tests, docs |

---

## Future Enhancements (Post-v1)

### Multi-Region Relays

- Deploy relays in multiple geographic regions
- Client automatically selects closest relay
- Improved latency for global users

### WebRTC P2P with Relay Fallback

- Attempt WebRTC direct connection first
- Use relay for TURN fallback
- Best of both worlds (low latency + reliability)

### Relay Mesh Network

- Relays can communicate with each other
- Route through multiple relays if needed
- Improved reliability and performance

### Advanced Token Features

- Token refresh (rotate before expiry)
- Scoped permissions (read-only, specific commands)
- Per-command rate limiting

---

## References

- MULTI_IP_ENROLLMENT.md: Multi-IP enrollment foundation
- FEATURE_IPV6_ENHANCEMENT.md: IPv6 support
- PRD.md: HandControl Product Requirements
- SECURITY.md: Security architecture
- RFC 8305: Happy Eyeballs (IPv6 connection preference)
- RFC 7519: JSON Web Token (JWT)

---

**Document Status:** Ready for implementation after Multi-IP and IPv6 features complete.

**Next Steps:**
1. Complete Multi-IP Enrollment implementation
2. Complete IPv6 Enhancement implementation
3. Begin Relay Server Foundation (Phase 1)
