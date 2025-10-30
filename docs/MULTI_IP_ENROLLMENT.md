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

1. **Primary IP**: Result from `get_local_ip()` (UDP connect trick to 8.8.8.8)
2. **Secondary IPs**: Other IPv4 addresses (sorted by interface name)
3. **Tertiary IPs**: IPv6 addresses (sorted by interface name)
4. **Deduplication**: Remove duplicates while preserving order

### 2. Protocol Changes (Breaking)

#### Proto File: `proto/handcontrol.proto`

**EnrollmentQrPayload** (in JSON, not protobuf):
```json
{
  "ips": ["10.83.20.105", "192.168.1.100", "fe80::1"],  // Changed from "ip"
  "port": 50051,
  "cert_fingerprint": "SHA256:...",
  "enrollment_token": "uuid",
  "server_id": "uuid"
}
```

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
}
```

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

### 6. Testing Strategy

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

### mDNS Integration

- Use mDNS to advertise all IPs
- Android client discovers via mDNS instead of QR
- QR becomes optional fallback

### IP Reachability Testing

- Server probes IPs before advertising
- Only include IPs that can reach internet
- Reduces failed connection attempts

### User IP Selection

- CLI flag: `--ip 192.168.1.100` to override detection
- Useful for complex network setups
- Helpful for debugging

### IPv6 Privacy Extensions

- Detect and prefer stable IPv6 addresses
- Exclude temporary privacy addresses
- Better for persistent connections

## Acceptance Criteria

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

## References

- [if-addrs crate](https://crates.io/crates/if-addrs)
- [RFC 3927 - Link-Local IPv4](https://tools.ietf.org/html/rfc3927)
- [RFC 4291 - IPv6 Addressing Architecture](https://tools.ietf.org/html/rfc4291)
- Project PRD: `docs/PRD.md`
- Security documentation: `docs/SECURITY.md`
