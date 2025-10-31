# Multi-IP Enrollment Feature Specification

## Overview

This document specifies the Multi-IP Enrollment feature for HandControl, which enhances the QR code enrollment and CLI commands to support multiple network interfaces with intelligent IP detection, filtering, and connection retry logic.

## Motivation

### Problem Statement

When the HandControl server binds to all interfaces (0.0.0.0 or ::), the current implementation:
- Only detects a single IP address using the `get_local_ip()` heuristic
- May select an IP that is unreachable from the Android device (e.g., VPN interface, Docker bridge)
- Provides no fallback mechanism if the detected IP is incorrect
- Does not leverage IPv6 when available
- Inconsistent IP detection across different CLI commands

### Goals

1. **Comprehensive Network Discovery**: Detect and provide all usable network interfaces (IPv4 and IPv6)
2. **Intelligent Filtering**: Exclude loopback, link-local, Docker, and VPN interfaces
3. **Smart Prioritization**: Order IPs by likelihood of success (get_local_ip() first)
4. **Robust Connection**: Hybrid retry strategy with short timeout on primary IP, parallel on fallbacks
5. **Consistency**: Apply same logic across all CLI commands that connect to the server
6. **Cross-Platform**: Work on Linux, macOS, Windows, and Android
7. **Relay-Ready Architecture**: Design with future cross-network relay support in mind

## Technical Design

### 1. Network Interface Enumeration

#### Dependency

Add `if-addrs` crate (pure Rust, cross-platform):
```toml
[dependencies]
if-addrs = "0.13"
```

#### New Module: `src/utils/network.rs`

```rust
/// Get all usable local IP addresses, filtered and prioritized
pub fn get_all_local_ips() -> Vec<String>

/// Get single best-guess local IP (existing logic)
pub fn get_local_ip() -> Option<String>

/// Check if an interface should be filtered out
fn should_filter_interface(name: &str) -> bool

/// Check if an IP should be filtered out
fn should_filter_ip(ip: &IpAddr) -> bool
```

#### Filtering Rules

**Exclude:**
- Loopback addresses: 127.0.0.0/8, ::1
- Link-local addresses: 169.254.0.0/16, fe80::/10
- Docker interfaces: docker0, br-*, veth*
- VPN interfaces: tun*, tap*, vpn*, wg*
- Interfaces that are DOWN

**Include:**
- All IPv4 addresses from UP interfaces
- All IPv6 addresses from UP interfaces (excluding temporary privacy addresses)

#### Prioritization Algorithm

**Current Phase (Local Network Direct Connections):**

1. **Primary IP**: Result from `get_local_ip()` (UDP connect trick to 8.8.8.8)
2. **Secondary IPs**: Other IPv4 addresses (sorted by interface name)
3. **Tertiary IPs**: IPv6 addresses (sorted by interface name)
4. **Deduplication**: Remove duplicates while preserving order

**Future Phase (With Relay Support):**

When relay information is available, the connection strategy becomes:

1. **Primary IP**: Try direct connection first (preferred for latency)
2. **Secondary IPs**: Parallel fallback to other direct IPs
3. **Relay Fallback**: Only if all direct connections fail and relay info present

Configuration options for relay behavior:
```toml
[network.relay]
prefer_relay = false           # If true, try relay before secondary IPs
relay_only_mode = false        # Skip direct connection attempts entirely
max_direct_attempts = 3        # Limit direct attempts before relay fallback
```

### 2. Protocol Changes (Breaking)

#### Proto File: `proto/handcontrol.proto`

**EnrollmentQrPayload** (in JSON, not protobuf):
```json
{
  "ips": ["10.83.20.105", "192.168.1.100", "fe80::1"],  // Changed from "ip"
  "port": 50051,
  "cert_fingerprint": "SHA256:...",
  "enrollment_token": "uuid",
  "server_id": "uuid",

  // Optional relay fields (future, relay-ready design)
  "relay": {
    "enabled": false,
    "url": "https://relay.example.com:50052",
    "token": "jwt-token-here"
  }
}
```

**Note:** The `relay` field is optional and reserved for future use. Current implementation ignores it. When relay support is added:
- Old clients without relay support ignore the field (JSON forward compatibility)
- New clients check for `relay.enabled` and use relay as fallback
- Backward compatibility maintained

**GenerateEnrollmentQRResponse**:
```protobuf
message GenerateEnrollmentQRResponse {
  bool success = 1;
  string qr_payload = 2;
  repeated string server_ips = 3;  // Changed from string server_ip = 4
  int32 server_port = 4;           // Renumbered
  string server_cert_fingerprint = 5;
  string server_id = 6;
  string enrollment_token = 7;
  int32 ttl_seconds = 8;
  string error_message = 9;

  // Reserved for future relay support (optional)
  optional RelayInfo relay_info = 10;
}

// Relay information (future use)
message RelayInfo {
  string relay_url = 1;           // Relay server URL
  string relay_token = 2;          // JWT token for relay authentication
  bool relay_required = 3;        // If true, skip direct connection attempts
}
```

**Relay Compatibility Notes:**
- `RelayInfo` is marked optional - old clients ignore unknown fields
- New clients check for relay_info presence before using relay
- Server can conditionally include relay_info based on configuration
- Proto definition reserves field number 10 for relay support

### 3. Rust Server Implementation

#### QR Payload Structure: `src/utils/qr.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentQrPayload {
    pub ips: Vec<String>,        // Changed from String
    pub port: u16,
    pub cert_fingerprint: String,
    pub enrollment_token: String,
    pub server_id: String,
}
```

**Display Changes**:
```
=== HandControl Enrollment QR Code ===

Scan this QR code with your Android device to enroll:

[QR CODE]

Primary Server: 10.83.20.105
Alternative IPs:
  - 192.168.1.100
  - fe80::1%eth0

Port: 50051
Server ID: 5a8af850-37c8-4e5c-977d-91b533c135c3
Token expires in 5 minutes
```

#### gRPC Server: `src/grpc/server.rs`

Update `generate_enrollment_qr()`:
```rust
async fn generate_enrollment_qr(
    &self,
    _request: Request<GenerateEnrollmentQrRequest>,
) -> Result<Response<GenerateEnrollmentQrResponse>, Status> {
    // Get all local IPs (filtered and prioritized)
    let server_ips = if self.config.server.bind_address == "0.0.0.0"
        || self.config.server.bind_address == "::"
    {
        let ips = get_all_local_ips();
        if ips.is_empty() {
            vec!["127.0.0.1".to_string()]
        } else {
            ips
        }
    } else {
        vec![self.config.server.bind_address.clone()]
    };

    // Create payload with multiple IPs
    let payload = EnrollmentQrPayload::new(
        server_ips.clone(),
        self.config.server.port,
        // ...
    );

    // Return response with repeated field
    Ok(Response::new(GenerateEnrollmentQrResponse {
        success: true,
        server_ips,  // repeated field
        // ...
    }))
}
```

### 4. Android Client Implementation

#### QR Scanner: `android/app/src/main/kotlin/com/handcontrol/feature/enrollment/QrScannerScreen.kt`

Update JSON parsing (around line 144):
```kotlin
val json = JSONObject(qrCode)

// Parse IPs array
val ipsArray = json.getJSONArray("ips")
val ips = mutableListOf<String>()
for (i in 0 until ipsArray.length()) {
    ips.add(ipsArray.getString(i))
}

if (ips.isEmpty()) {
    throw IllegalArgumentException("QR code contains no valid IP addresses")
}

val port = json.getInt("port")
val token = json.getString("enrollment_token")
val fingerprint = json.getString("cert_fingerprint")
val serverId = json.getString("server_id")
```

#### Enrollment Repository: `android/app/src/main/kotlin/com/handcontrol/data/enrollment/GrpcEnrollmentRepository.kt`

Implement hybrid connection strategy:
```kotlin
suspend fun enrollWithToken(
    hosts: List<String>,  // Changed from single host
    port: Int,
    enrollmentToken: String,
    clientCert: ByteArray,
    deviceName: String,
    deviceModel: String
): Result<String> = withContext(Dispatchers.IO) {
    require(hosts.isNotEmpty()) { "At least one host is required" }

    // Try primary IP with short timeout
    val primaryHost = hosts.first()
    try {
        return@withContext tryEnrollWithHost(
            host = primaryHost,
            port = port,
            enrollmentToken = enrollmentToken,
            clientCert = clientCert,
            deviceName = deviceName,
            deviceModel = deviceModel,
            timeoutSeconds = 2
        )
    } catch (e: Exception) {
        Timber.d("Primary host $primaryHost failed: ${e.message}")
    }

    // If primary fails, try remaining IPs in parallel
    if (hosts.size == 1) {
        return@withContext Result.failure(
            IOException("Failed to connect to $primaryHost")
        )
    }

    val remainingHosts = hosts.drop(1)
    return@withContext coroutineScope {
        val results = remainingHosts.map { host ->
            async {
                tryEnrollWithHost(
                    host = host,
                    port = port,
                    enrollmentToken = enrollmentToken,
                    clientCert = clientCert,
                    deviceName = deviceName,
                    deviceModel = deviceModel,
                    timeoutSeconds = 5
                )
            }
        }

        // Return first successful result
        try {
            results.awaitFirstSuccess()
        } catch (e: Exception) {
            Result.failure(
                IOException("Failed to connect to any host: ${hosts.joinToString()}")
            )
        }
    }
}

private suspend fun tryEnrollWithHost(
    host: String,
    port: Int,
    enrollmentToken: String,
    clientCert: ByteArray,
    deviceName: String,
    deviceModel: String,
    timeoutSeconds: Long
): Result<String> = withTimeout(timeoutSeconds * 1000) {
    // Existing enrollment logic
}

// Extension function to await first success
private suspend fun <T> List<Deferred<Result<T>>>.awaitFirstSuccess(): Result<T> {
    val errors = mutableListOf<Throwable>()

    for (deferred in this) {
        try {
            val result = deferred.await()
            if (result.isSuccess) {
                // Cancel remaining tasks
                forEach { if (it != deferred) it.cancel() }
                return result
            }
            result.exceptionOrNull()?.let { errors.add(it) }
        } catch (e: Exception) {
            errors.add(e)
        }
    }

    return Result.failure(IOException("All hosts failed: $errors"))
}
```

#### Channel Factory: `android/app/src/main/kotlin/com/handcontrol/core/network/MtlsGrpcChannelFactory.kt`

Add timeout support:
```kotlin
fun createChannel(
    host: String,
    port: Int,
    clientCert: ByteArray,
    clientKey: ByteArray,
    serverCertFingerprint: ByteArray,
    timeoutSeconds: Long = 10  // New parameter
): ManagedChannel {
    // Existing mTLS setup...

    return OkHttpChannelBuilder
        .forAddress(host, port)
        .sslSocketFactory(sslContext.socketFactory)
        // Add connection timeout
        .keepAliveTime(timeoutSeconds, TimeUnit.SECONDS)
        .keepAliveTimeout(timeoutSeconds, TimeUnit.SECONDS)
        .build()
}
```

#### Connection Mode Abstraction (Relay-Ready Design)

**File:** `android/app/src/main/kotlin/com/handcontrol/core/network/ConnectionStrategy.kt` (NEW - Future)

Prepare for multiple connection modes by abstracting the connection logic:

```kotlin
// Connection attempt types
sealed class ConnectionAttempt {
    data class Direct(
        val hosts: List<String>,
        val port: Int,
        val tlsConfig: TlsConfig
    ) : ConnectionAttempt()

    data class Relay(
        val relayUrl: String,
        val serverId: String,
        val relayToken: String,
        val tlsConfig: TlsConfig
    ) : ConnectionAttempt()
}

// Strategy interface for different connection modes
interface ConnectionStrategy {
    suspend fun connect(attempt: ConnectionAttempt): Result<ManagedChannel>
}

// Direct connection implementation (current)
class DirectConnectionStrategy(
    private val channelFactory: MtlsGrpcChannelFactory
) : ConnectionStrategy {
    override suspend fun connect(attempt: ConnectionAttempt): Result<ManagedChannel> {
        return when (attempt) {
            is ConnectionAttempt.Direct -> connectDirect(attempt)
            else -> Result.failure(UnsupportedOperationException("Relay not yet implemented"))
        }
    }

    private suspend fun connectDirect(attempt: ConnectionAttempt.Direct): Result<ManagedChannel> {
        // Existing multi-IP connection logic
        // Try primary IP, then fallback IPs in parallel
    }
}

// Relay connection implementation (future)
class RelayConnectionStrategy(
    private val channelFactory: MtlsGrpcChannelFactory
) : ConnectionStrategy {
    override suspend fun connect(attempt: ConnectionAttempt): Result<ManagedChannel> {
        return when (attempt) {
            is ConnectionAttempt.Relay -> connectViaRelay(attempt)
            else -> Result.failure(UnsupportedOperationException("Direct connection not supported by relay strategy"))
        }
    }

    private suspend fun connectViaRelay(attempt: ConnectionAttempt.Relay): Result<ManagedChannel> {
        // Future implementation:
        // 1. Connect to relay server
        // 2. Send connection request with relay_token and server_id
        // 3. Establish tunnel through relay
        // 4. Return channel that proxies through relay
        throw NotImplementedError("Relay support coming in future release")
    }
}

// Smart connection manager that tries multiple strategies
class SmartConnectionManager(
    private val directStrategy: DirectConnectionStrategy,
    private val relayStrategy: RelayConnectionStrategy? = null  // Optional for now
) {
    suspend fun connect(
        directHosts: List<String>,
        port: Int,
        tlsConfig: TlsConfig,
        relayInfo: RelayInfo? = null
    ): Result<ManagedChannel> {
        // Try direct connection first (preferred for latency)
        val directAttempt = ConnectionAttempt.Direct(directHosts, port, tlsConfig)
        val directResult = directStrategy.connect(directAttempt)

        if (directResult.isSuccess) {
            return directResult
        }

        // If direct fails and relay available, try relay
        if (relayInfo != null && relayInfo.enabled && relayStrategy != null) {
            val relayAttempt = ConnectionAttempt.Relay(
                relayUrl = relayInfo.url,
                serverId = relayInfo.serverId,
                relayToken = relayInfo.token,
                tlsConfig = tlsConfig
            )
            return relayStrategy.connect(relayAttempt)
        }

        // No relay available or relay failed
        return directResult
    }
}
```

**Benefits of This Abstraction:**
- **Clean separation**: Direct vs relay logic isolated
- **Easy testing**: Mock connection strategies independently
- **Future-proof**: Add relay without changing enrollment code
- **Flexible**: Can add more connection modes (WebRTC P2P, etc.)
- **Gradual rollout**: Relay strategy optional, can be added incrementally

#### Database Schema (Relay-Ready)

**File:** `android/app/src/main/kotlin/com/handcontrol/core/data/local/EnrolledServer.kt`

Update the database entity to support future relay information:

```kotlin
@Entity(tableName = "enrolled_servers")
data class EnrolledServer(
    @PrimaryKey val serverId: String,
    val serverName: String,
    val ips: List<String>,              // NEW: Multiple IPs (was single 'host')
    val port: Int,
    val clientId: String,
    val certFingerprint: String,
    val enrolledAt: Long,
    val lastConnectedAt: Long? = null,

    // Relay support fields (optional, for future use)
    val relayEnabled: Boolean = false,
    val relayUrl: String? = null,
    val relayToken: String? = null,
    val lastConnectionMode: ConnectionMode = ConnectionMode.DIRECT,  // Track which mode succeeded

    // Deprecated but kept for migration
    @Deprecated("Use ips instead")
    val host: String? = null
)

// Connection mode tracking
enum class ConnectionMode {
    DIRECT,      // Connected via direct IP
    RELAY,       // Connected via relay server
    UNKNOWN      // Unknown/not yet connected
}
```

**Room Type Converters:**

```kotlin
class Converters {
    @TypeConverter
    fun fromStringList(value: List<String>): String {
        return value.joinToString(",")
    }

    @TypeConverter
    fun toStringList(value: String): List<String> {
        return if (value.isEmpty()) emptyList() else value.split(",")
    }

    @TypeConverter
    fun fromConnectionMode(value: ConnectionMode): String {
        return value.name
    }

    @TypeConverter
    fun toConnectionMode(value: String): ConnectionMode {
        return try {
            ConnectionMode.valueOf(value)
        } catch (e: IllegalArgumentException) {
            ConnectionMode.UNKNOWN
        }
    }
}
```

**Database Migration:**

```kotlin
// Migration from version 1 (single host) to version 2 (multi-IP + relay)
val MIGRATION_1_2 = object : Migration(1, 2) {
    override fun migrate(database: SupportSQLiteDatabase) {
        // Add new columns
        database.execSQL("ALTER TABLE enrolled_servers ADD COLUMN ips TEXT NOT NULL DEFAULT ''")
        database.execSQL("ALTER TABLE enrolled_servers ADD COLUMN relayEnabled INTEGER NOT NULL DEFAULT 0")
        database.execSQL("ALTER TABLE enrolled_servers ADD COLUMN relayUrl TEXT")
        database.execSQL("ALTER TABLE enrolled_servers ADD COLUMN relayToken TEXT")
        database.execSQL("ALTER TABLE enrolled_servers ADD COLUMN lastConnectionMode TEXT NOT NULL DEFAULT 'UNKNOWN'")

        // Migrate existing host values to ips (backward compatibility)
        database.execSQL("UPDATE enrolled_servers SET ips = host WHERE host IS NOT NULL AND host != ''")
    }
}
```

**Benefits:**
- **Prepared for relay**: Database already has relay fields (unused until relay feature added)
- **Backward compatible**: Migration preserves existing data
- **Connection tracking**: Can optimize future connections based on what worked last time
- **Clean rollout**: Relay fields optional, no impact on current functionality

### 5. CLI Commands Consistency

Apply same multi-IP logic to all CLI commands that connect to the server:

#### Commands to Update:
- `handle_enroll_command()` - Already uses the RPC
- `handle_approve_command()` - Currently uses single IP
- `handle_list_pending_command()` - Currently uses single IP
- `handle_reject_command()` - Currently uses single IP

#### Implementation Pattern:

```rust
async fn connect_to_server(config: &Config) -> Result<RemoteControlClient<Channel>> {
    let host = match config.server.bind_address.as_str() {
        "0.0.0.0" | "::" => {
            // Try to connect using prioritized IPs
            let ips = get_all_local_ips();
            let primary = ips.first()
                .cloned()
                .unwrap_or_else(|| "127.0.0.1".to_string());
            primary
        }
        other => other.to_string(),
    };

    // Existing connection logic...
}
```

### 6. Network Topology Support

#### Current Support (Local Network Direct Connections)

This multi-IP feature focuses on improving connectivity within the same local network:

**Supported Scenarios:**
- **Same subnet** (client and server on same WiFi network)
- **Multiple local networks** (server connected to WiFi + Ethernet)
- **IPv4 and IPv6 dual-stack** environments
- **Direct peer-to-peer connectivity** (no middleboxes)

**Example Topology:**
```
Home WiFi Network (192.168.1.0/24, 2001:db8::/64)
    |
    +--- Android Phone (192.168.1.50, 2001:db8::50)
    |
    +--- PC Server (192.168.1.100, 2001:db8::100)
          Also connected to Ethernet (10.0.0.100)

Connection attempt order:
1. Try 192.168.1.100 (WiFi, same subnet) - FAST
2. Try 10.0.0.100 (Ethernet, different subnet) - May fail if routing not configured
3. Try 2001:db8::100 (IPv6) - Works if IPv6 enabled
```

#### Future Support (Cross-Network via Relay)

When relay support is added, HandControl will work across different networks:

**New Scenarios Enabled:**
- **Different subnets** (home network vs mobile network)
- **Behind NAT/firewalls** (client and server not directly routable)
- **IPv6-only client to IPv4-only server** (relay acts as protocol bridge)
- **Long-distance connections** (different geographical locations)
- **Cellular networks** (mobile data without VPN)

**Example Cross-Network Topology:**
```
Home Network (Private: 192.168.1.0/24)                Mobile Network (Carrier NAT: 10.x.x.x)
    |                                                      |
    +--- PC Server (192.168.1.100)                        +--- Android Phone (10.x.x.x)
          |                                                      |
          | [Direct connection fails - different networks]      |
          |                                                      |
          +--- Connect to Relay -----> [Cloud Relay] <---- Connect to Relay
                                       (Public IP)
                                           |
                                           +--- Tunnel packets between
                                                client and server
                                                (end-to-end mTLS maintained)
```

**Connection Decision Tree (Future):**

```
Start Connection Attempt
    |
    +--- Has direct IPs? ---+
    |                       |
    | YES                   | NO
    |                       |
    +---> Try direct IPs    +---> Has relay info? ---+
          |                                          |
          | Success?                                 | YES
          | YES --> Done (FAST)                      |
          | NO --> Failed                            +---> Try relay connection
          |                                                |
          +---> Has relay info? ---+                       | Success?
                                   |                       | YES --> Done (SLOWER but works)
                                   | YES                   | NO --> Error (no connectivity)
                                   |
                                   +---> Try relay as fallback
                                         |
                                         | Success? --> Done (fallback worked)
                                         | NO --> Error (all methods failed)
```

**Design Principles:**
1. **Direct first**: Always prefer direct connection (lower latency, no infrastructure cost)
2. **Relay as fallback**: Only use relay when direct fails
3. **User control**: Configuration options to prefer/force relay if needed
4. **Transparent**: User doesn't need to understand topology, connection "just works"

### 7. Testing Strategy

#### Rust Unit Tests

```rust
#[test]
fn test_get_all_local_ips() {
    let ips = get_all_local_ips();
    assert!(!ips.is_empty());

    // Verify no loopback
    assert!(!ips.iter().any(|ip| ip.starts_with("127.")));

    // Verify prioritization (IPv4 before IPv6)
    if ips.len() > 1 {
        let first_is_v4 = ips[0].contains('.');
        let last_is_v6 = ips.last().unwrap().contains(':');
        // If we have both, IPv4 should come first
    }
}

#[test]
fn test_filter_docker_interfaces() {
    assert!(should_filter_interface("docker0"));
    assert!(should_filter_interface("br-1234567890ab"));
    assert!(should_filter_interface("veth1a2b3c4"));
    assert!(!should_filter_interface("eth0"));
    assert!(!should_filter_interface("wlan0"));
}
```

#### Android Instrumentation Tests

```kotlin
@Test
fun testMultiIpConnectionRetry() = runTest {
    val hosts = listOf(
        "192.0.2.1",      // Unreachable (TEST-NET-1)
        "127.0.0.1"       // Should work
    )

    val result = repository.enrollWithToken(
        hosts = hosts,
        port = 50051,
        // ...
    )

    assertTrue(result.isSuccess)
}

@Test
fun testAllHostsFail() = runTest {
    val hosts = listOf(
        "192.0.2.1",
        "192.0.2.2",
        "192.0.2.3"
    )

    val result = repository.enrollWithToken(
        hosts = hosts,
        port = 50051,
        // ...
    )

    assertTrue(result.isFailure)
}
```

#### Manual Testing Scenarios

1. **Single IPv4 Network**
   - Server on WiFi only
   - Verify correct IP detection

2. **Multiple IPv4 Networks**
   - Server on WiFi + Ethernet
   - Verify prioritization

3. **IPv6 Network**
   - Server on IPv6-enabled network
   - Verify IPv6 addresses included

4. **Docker/VPN Present**
   - Server with Docker installed
   - Verify Docker IPs filtered out

5. **Unreachable Primary IP**
   - Mock first IP as unreachable
   - Verify fallback to secondary

6. **All IPs Unreachable**
   - All IPs timing out
   - Verify proper error message

#### Future Testing Scenarios (With Relay Support)

These tests will be added when relay functionality is implemented:

**1. Relay Fallback on Direct Failure**
- Block all direct IP connections (firewall rules)
- Provide valid relay information
- Verify client falls back to relay automatically
- Confirm end-to-end mTLS maintained through relay
- Measure connection establishment time

**2. Relay Performance Comparison**
- Connect via direct IP and measure latency/bandwidth
- Connect via relay and measure latency/bandwidth
- Compare: Direct should be faster, relay should work
- Document acceptable performance degradation

**3. Relay Unavailable Handling**
- Provide relay info with unreachable relay server
- Verify clear error message (not generic timeout)
- Verify no infinite retry loops
- Test exponential backoff behavior

**4. Relay Token Expiry**
- Enroll with valid relay token
- Wait for token to expire
- Attempt connection via relay
- Verify graceful failure with appropriate error
- Test token refresh flow (if implemented)

**5. Relay Priority Configuration**
- Test `prefer_relay = true` config
- Verify relay tried before fallback IPs
- Test `relay_only_mode = true` config
- Verify direct IPs skipped entirely

**6. Connection Mode Persistence**
- Connect via direct, verify `lastConnectionMode = DIRECT`
- Force relay connection, verify `lastConnectionMode = RELAY`
- Next connection should prefer last successful mode
- Test mode fallback if preferred mode fails

## Migration Guide

### Server Upgrade Path

1. **Deploy new server** with multi-IP support
2. **Old Android clients will fail** when scanning new QR codes
3. Users must update Android app before using new enrollment

### Breaking Changes Checklist

- [ ] Update API documentation
- [ ] Increment API version in server response
- [ ] Update Android app minimum version requirement
- [ ] Add migration notes to CHANGELOG.md
- [ ] Update enrollment documentation

## Performance Considerations

### Network Enumeration Cost

- `if_addrs::get_if_addrs()` is fast (< 1ms on typical systems)
- Called once per QR generation (not on hot path)
- No performance impact

### Android Connection Overhead

- Primary IP: 2-second timeout (same as current)
- Fallback IPs: Parallel connections (faster than sequential)
- Worst case: 2s + 5s = 7 seconds total
- Best case: 2 seconds (same as current)
- Average case: 2-4 seconds

### Memory Impact

- Storing 5-10 IP strings: negligible (< 500 bytes)
- Parallel Deferred tasks: ~2KB per connection
- Total: < 10KB additional memory

## Security Considerations

### Information Disclosure

**Risk**: QR code exposes all server IP addresses

**Mitigation**:
- QR codes are shown on trusted devices (the server itself)
- Physical/visual access already implies network access
- No new attack surface introduced

### Man-in-the-Middle

**Protection**: Existing mTLS with certificate fingerprint validation
- Each IP attempt validates server cert fingerprint
- MITM cannot impersonate without private key
- No degradation in security posture

### Denial of Service

**Risk**: Client tries many IPs, consuming resources

**Mitigation**:
- Limited to actual network interfaces (typically < 10)
- Short timeouts prevent resource exhaustion
- Parallel connections use bounded concurrency

## Future Enhancements

### Local Network Improvements

#### mDNS Integration
- Use mDNS to advertise all IPs
- Android client discovers via mDNS instead of QR
- QR becomes optional fallback

#### IP Reachability Testing
- Server probes IPs before advertising
- Only include IPs that can reach internet
- Reduces failed connection attempts

#### User IP Selection
- CLI flag: `--ip 192.168.1.100` to override detection
- Useful for complex network setups
- Helpful for debugging

#### IPv6 Privacy Extensions
- Detect and prefer stable IPv6 addresses
- Exclude temporary privacy addresses
- Better for persistent connections

### Cross-Network Relay Support

#### Relay Server Infrastructure
- Standalone relay server binary (separate crate)
- Deploy to cloud VPS or self-hosted infrastructure
- Support multiple HandControl servers per relay
- JWT-based authentication and authorization
- Rate limiting and abuse prevention
- Health monitoring and metrics

#### Server-Side Relay Integration
- Configuration: relay server URL and auth credentials
- Auto-connect to relay on server startup
- Generate relay tokens for enrolled clients
- Include relay info in QR codes (optional field)
- Maintain persistent tunnel to relay server
- Automatic reconnection on relay disconnect

#### Client-Side Relay Support
- Parse relay info from QR codes
- Store relay credentials in database
- Implement `RelayConnectionStrategy`
- Automatic fallback: direct -> relay
- Connection mode tracking and optimization
- UI indicator for connection type (direct/relay)

#### Relay Protocol Design
- gRPC bidirectional streaming for tunnel
- End-to-end mTLS preserved through relay
- Relay is transport-layer only (no decryption)
- Efficient packet forwarding
- Connection multiplexing support

#### Configuration Options
```toml
[relay]
enabled = false
relay_server_url = "https://relay.example.com:50052"
relay_auth_secret = "server-secret"
auto_connect = true
# allow_self_signed_tls = true
# pinned_cert_sha256 = "AA...FF"

[relay.client]
prefer_relay = false        # Try relay before fallback IPs
relay_only_mode = false     # Skip direct connections
max_direct_attempts = 3     # Limit before relay fallback
```

**See `docs/FEATURE_RELAY.md` for complete relay implementation plan.**

## Acceptance Criteria

### Current Phase (Multi-IP Local Network)

- [ ] Server detects all usable IPv4 and IPv6 addresses
- [ ] Docker/VPN/loopback interfaces are filtered out
- [ ] IPs are prioritized: primary, IPv4, IPv6
- [ ] QR code displays all IPs clearly
- [ ] Android client tries primary IP first with 2s timeout
- [ ] Android client tries remaining IPs in parallel if primary fails
- [ ] All CLI commands use consistent IP detection
- [ ] Unit tests pass for network enumeration
- [ ] Integration tests pass for multi-IP enrollment
- [ ] Manual testing completed on WiFi, Ethernet, IPv6 networks
- [ ] Documentation updated
- [ ] Breaking changes documented in CHANGELOG

### Future Phase Preparation (Relay-Ready)

These criteria ensure the codebase is prepared for relay support without implementing it yet:

- [ ] Proto definitions include `RelayInfo` message (optional, reserved)
- [ ] QR payload JSON structure supports optional `relay` object
- [ ] Android database schema includes relay fields (unused)
- [ ] Database migration tested and documented
- [ ] Connection strategy abstraction implemented (`ConnectionStrategy` interface)
- [ ] `SmartConnectionManager` structure prepared for multiple connection modes
- [ ] Configuration files include `[relay]` section (disabled by default)
- [ ] `ConnectionMode` enum exists and is persisted in database
- [ ] All relay code paths throw `NotImplementedError` with clear messages
- [ ] Documentation references `FEATURE_RELAY.md` for future implementation

## References

- [if-addrs crate](https://crates.io/crates/if-addrs)
- [RFC 3927 - Link-Local IPv4](https://tools.ietf.org/html/rfc3927)
- [RFC 4291 - IPv6 Addressing Architecture](https://tools.ietf.org/html/rfc4291)
- Project PRD: `docs/PRD.md`
- Security documentation: `docs/SECURITY.md`
