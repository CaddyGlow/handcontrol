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
6. **JWT Authentication**: Server-issued tokens, relay validates

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
│         │           │   :50052            │               │         │
│         │           └──────────┬──────────┘               │         │
│         │                      │                           │         │
│         ├──────────────────────┤                           │         │
│         │  gRPC Stream         │  gRPC Stream              │         │
│         │  (mTLS)              │  (mTLS)                   │         │
│         │                      │                           │         │
│    [Encrypted Data]──────>[Relay Proxy]──────>[Encrypted Data]      │
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
relay_server_url = "https://relay.example.com:50052"
relay_auth_secret = "base64-encoded-secret"
auto_connect = true
include_in_enrollment = true
```

#### 3. Android Client (Updated)

**New Capabilities:**
- Parse relay info from QR codes
- Store relay credentials in database
- Implement relay connection strategy
- Automatic fallback (direct -> relay)
- Connection mode tracking
- UI indicator for connection type

### Relay Protocol Design

#### Connection Sequence

```
Server Registration Flow:
1. Server connects to relay
2. Server sends RegisterRequest(server_id, relay_secret)
3. Relay validates secret
4. Relay keeps connection open (bidirectional stream)
5. Server is now "registered" and can accept client connections

Client Connection Flow:
1. Client tries direct IPs (multi-IP, IPv6 first)
2. If all fail and relay info available:
   a. Client connects to relay
   b. Client sends ConnectRequest(server_id, relay_token)
   c. Relay validates JWT token
   d. Relay finds registered server by server_id
   e. Relay creates tunnel: client <-> relay <-> server
3. All subsequent gRPC calls flow through relay tunnel
4. End-to-end mTLS maintained
```

#### Proto Definitions

**File:** `proto/handcontrol.proto` (additions)

```protobuf
// Relay service (runs on relay server)
service RelayService {
  // Server registers to accept client connections
  rpc RegisterServer(stream ServerRelayMessage) returns (stream RelayControlMessage);

  // Client connects to registered server
  rpc ConnectToServer(stream ClientRelayMessage) returns (stream ServerRelayMessage);
}

// Server -> Relay messages
message ServerRelayMessage {
  oneof message {
    RegisterServerRequest register = 1;
    bytes data = 2;              // Encrypted gRPC data from server
    Ping ping = 3;
    Pong pong = 4;
  }
}

message RegisterServerRequest {
  string server_id = 1;
  string relay_secret = 2;       // Server's authentication secret
  string server_version = 3;
  repeated string capabilities = 4;
}

// Relay -> Server messages
message RelayControlMessage {
  oneof message {
    RegisterServerResponse register_response = 1;
    ClientConnectedNotification client_connected = 2;
    bytes data = 3;              // Encrypted gRPC data from client
    Ping ping = 4;
    Pong pong = 5;
  }
}

message RegisterServerResponse {
  bool success = 1;
  string error_message = 2;
}

message ClientConnectedNotification {
  string client_id = 1;
  string tunnel_id = 2;          // Unique ID for this tunnel
}

// Client -> Relay messages
message ClientRelayMessage {
  oneof message {
    ConnectToServerRequest connect = 1;
    bytes data = 2;              // Encrypted gRPC data from client
    Ping ping = 3;
    Pong pong = 4;
  }
}

message ConnectToServerRequest {
  string server_id = 1;
  string relay_token = 2;        // JWT token issued by server
}

// Relay -> Client messages (reuse ServerRelayMessage with different semantics)

// Keep-alive messages
message Ping {
  int64 timestamp_ms = 1;
}

message Pong {
  int64 timestamp_ms = 1;
}
```

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
use tonic::{transport::Server, Request, Response, Status};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
struct RelayState {
    // Map: server_id -> ServerConnection
    registered_servers: Arc<RwLock<HashMap<String, ServerConnection>>>,

    // Map: tunnel_id -> Tunnel
    active_tunnels: Arc<RwLock<HashMap<String, Tunnel>>>,
}

struct ServerConnection {
    server_id: String,
    tx: mpsc::Sender<RelayControlMessage>,
}

struct Tunnel {
    tunnel_id: String,
    server_id: String,
    client_tx: mpsc::Sender<ServerRelayMessage>,
    server_tx: mpsc::Sender<RelayControlMessage>,
}

#[tonic::async_trait]
impl RelayService for RelayServiceImpl {
    async fn register_server(
        &self,
        request: Request<tonic::Streaming<ServerRelayMessage>>,
    ) -> Result<Response<Self::RegisterServerStream>, Status> {
        // Implementation
    }

    async fn connect_to_server(
        &self,
        request: Request<tonic::Streaming<ClientRelayMessage>>,
    ) -> Result<Response<Self::ConnectToServerStream>, Status> {
        // Implementation
    }
}
```

#### 1.3 Authentication

**File:** `relay/src/auth.rs`

```rust
use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};

pub struct TokenValidator {
    server_public_keys: HashMap<String, DecodingKey>,
}

impl TokenValidator {
    pub fn validate_relay_token(&self, token: &str) -> Result<Claims, AuthError> {
        // 1. Decode JWT header to get server_id
        // 2. Look up server's public key
        // 3. Validate signature
        // 4. Check expiry
        // 5. Verify claims (aud, iss, etc.)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub iss: String,
    pub sub: String,        // client_id
    pub aud: String,
    pub exp: u64,
    pub iat: u64,
    pub server_id: String,
    pub permissions: Vec<String>,
}
```

#### 1.4 Configuration

**File:** `relay/config.toml`

```toml
[relay]
bind_address = "::"
port = 50052
tls_cert_path = "/etc/handcontrol-relay/cert.pem"
tls_key_path = "/etc/handcontrol-relay/key.pem"

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

```rust
pub struct RelayClient {
    config: RelayConfig,
    server_id: Uuid,
    channel: Option<Channel>,
    state: Arc<RwLock<RelayClientState>>,
}

enum RelayClientState {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

impl RelayClient {
    pub async fn connect(&mut self) -> Result<()> {
        let channel = create_channel(&self.config.relay_server_url?)?;
        let mut client = RelayServiceClient::new(channel);

        let (tx, rx) = mpsc::channel(32);

        // Start bidirectional stream
        let stream = client.register_server(ReceiverStream::new(rx)).await?;

        // Send initial registration
        tx.send(ServerRelayMessage {
            message: Some(server_relay_message::Message::Register(
                RegisterServerRequest {
                    server_id: self.server_id.to_string(),
                    relay_secret: self.config.relay_auth_secret.clone()?,
                    server_version: env!("CARGO_PKG_VERSION").to_string(),
                    capabilities: vec!["v1".to_string()],
                }
            ))
        }).await?;

        // Handle incoming messages
        self.handle_relay_messages(stream, tx).await
    }

    async fn handle_relay_messages(
        &mut self,
        mut stream: Streaming<RelayControlMessage>,
        tx: mpsc::Sender<ServerRelayMessage>,
    ) -> Result<()> {
        while let Some(msg) = stream.message().await? {
            match msg.message {
                Some(relay_control_message::Message::RegisterResponse(resp)) => {
                    if resp.success {
                        tracing::info!("Successfully registered with relay server");
                        *self.state.write().await = RelayClientState::Connected;
                    } else {
                        tracing::error!("Relay registration failed: {}", resp.error_message);
                        return Err(anyhow!("Registration failed"));
                    }
                }
                Some(relay_control_message::Message::ClientConnected(notif)) => {
                    tracing::info!("Client connected via relay: {}", notif.client_id);
                    // Handle client connection through tunnel
                }
                Some(relay_control_message::Message::Data(data)) => {
                    // Forward decrypted gRPC data to local gRPC server
                }
                _ => {}
            }
        }
        Ok(())
    }
}
```

#### 2.3 Token Generation

**File:** `server/src/relay/tokens.rs` (NEW)

```rust
use jsonwebtoken::{encode, EncodingKey, Header, Algorithm};
use ed25519_dalek::{SigningKey, VerifyingKey};

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
            &EncodingKey::from_ed25519_der(&self.signing_key.to_bytes()),
        )?;

        Ok(token)
    }

    pub fn get_public_key_pem(&self) -> String {
        // Export verifying key for relay server
        pem::encode(&pem::Pem {
            tag: "PUBLIC KEY".to_string(),
            contents: self.verifying_key.to_bytes().to_vec(),
        })
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
    private val channelFactory: MtlsGrpcChannelFactory
) : ConnectionStrategy {

    override suspend fun connect(attempt: ConnectionAttempt): Result<ManagedChannel> {
        return when (attempt) {
            is ConnectionAttempt.Relay -> connectViaRelay(attempt)
            else -> Result.failure(UnsupportedOperationException())
        }
    }

    private suspend fun connectViaRelay(attempt: ConnectionAttempt.Relay): Result<ManagedChannel> = withContext(Dispatchers.IO) {
        try {
            // 1. Create connection to relay server
            val relayChannel = channelFactory.createChannel(
                host = extractHost(attempt.relayUrl),
                port = extractPort(attempt.relayUrl),
                tlsConfig = attempt.tlsConfig
            )

            val relayClient = RelayServiceClient(relayChannel)

            // 2. Create bidirectional stream
            val (requestChannel, responseFlow) = relayClient.connectToServer()

            // 3. Send connect request with token
            requestChannel.send(ClientRelayMessage.newBuilder()
                .setConnect(ConnectToServerRequest.newBuilder()
                    .setServerId(attempt.serverId)
                    .setRelayToken(attempt.relayToken)
                    .build())
                .build())

            // 4. Wait for connection confirmation
            val firstResponse = withTimeout(5000) {
                responseFlow.first()
            }

            // Validate connection established
            // ... validation logic ...

            // 5. Create tunneling channel that wraps relay stream
            val tunnelingChannel = TunnelingChannel(
                relayChannel = relayChannel,
                requestChannel = requestChannel,
                responseFlow = responseFlow
            )

            Result.success(tunnelingChannel)
        } catch (e: Exception) {
            Timber.e(e, "Relay connection failed")
            Result.failure(e)
        }
    }
}
```

#### 3.3 Tunneling Channel Implementation

**File:** `app/src/main/kotlin/com/handcontrol/core/network/TunnelingChannel.kt` (NEW)

```kotlin
/**
 * A ManagedChannel that tunnels gRPC calls through a relay server.
 *
 * This wraps the relay bidirectional stream and makes it transparent
 * to the rest of the application - it looks like a regular gRPC channel.
 */
class TunnelingChannel(
    private val relayChannel: ManagedChannel,
    private val requestChannel: SendChannel<ClientRelayMessage>,
    private val responseFlow: Flow<ServerRelayMessage>
) : ManagedChannel() {

    override fun <RequestT, ResponseT> newCall(
        methodDescriptor: MethodDescriptor<RequestT, ResponseT>,
        callOptions: CallOptions
    ): ClientCall<RequestT, ResponseT> {
        return TunnelingClientCall(
            methodDescriptor = methodDescriptor,
            requestChannel = requestChannel,
            responseFlow = responseFlow
        )
    }

    // Delegate lifecycle methods to relay channel
    override fun shutdown(): ManagedChannel = relayChannel.shutdown()
    override fun isShutdown(): Boolean = relayChannel.isShutdown
    override fun isTerminated(): Boolean = relayChannel.isTerminated
    override fun shutdownNow(): ManagedChannel = relayChannel.shutdownNow()
    override fun awaitTermination(timeout: Long, unit: TimeUnit): Boolean =
        relayChannel.awaitTermination(timeout, unit)
    override fun authority(): String = relayChannel.authority()
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
let addr = "[::]:50052".parse()?;
let socket = bind_tcp_listener(addr)?;
```

#### 4.2 IPv6 Relay URLs

Support IPv6 literal addresses in relay URLs:

```
https://[2001:db8::1]:50052
https://relay.example.com:50052  (resolves to IPv4 or IPv6)
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
  -p 50052:50052 \
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
- Requires port forwarding (50052)
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
   - Relay only sees encrypted bytes
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
- Public key: Shared with relay server (registration or config)

**Rotation:**
1. Generate new keypair
2. Register new public key with relay
3. Update server config
4. Issue new tokens to clients (gradual rollout)
5. Decommission old key after all clients updated

#### Relay Secrets

**Server Registration Secret:**
- Random 32-byte value
- Base64-encoded
- Shared between server and relay (out-of-band)
- Used only for initial registration

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
