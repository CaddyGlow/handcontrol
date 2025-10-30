# Feature Plan: IPv6 Enhancement

**Version:** 1.0
**Date:** 2025-10-30
**Status:** Planned
**Priority:** High
**Dependencies:** Multi-IP Enrollment Support (MULTI_IP_ENROLLMENT.md)

## Overview

Enhance HandControl with comprehensive IPv6 support to leverage native IPv6 connectivity, enabling better end-to-end reachability and preparing for IPv6-only networks. This feature builds on the multi-IP enrollment foundation to provide optimized IPv6 connectivity for both local and future cross-network scenarios.

## Goals

- **Primary**: Enable direct IPv6 connectivity between Android client and PC server
- **Secondary**: Optimize address selection to prefer IPv6 when available
- **Tertiary**: Prepare foundation for cross-network relay with IPv6 support
- Support both native IPv6 networks and dual-stack environments
- Maintain backward compatibility with IPv4-only networks

## Non-Goals (Deferred to Future Phases)

- IPv6-only operation (dual-stack required for v1)
- NAT64/DNS64 gateway configuration
- IPv6 transition mechanisms (6to4, Teredo, etc.)
- Relay server implementation (separate feature)

---

## Current State Analysis

### What Already Works

**Server Side:**
- Server binds to `::` (dual-stack) by default
- `socket2` crate properly configured with `set_only_v6(false)`
- Can accept both IPv4 and IPv6 connections on same socket
- File: `src/grpc/server.rs:860-890`

**What's Missing:**
- Multi-IP enumeration (only returns single IP)
- IPv6 address filtering and prioritization
- IPv6 scope handling (link-local vs global)
- IPv6 privacy extensions awareness
- IPv6 address testing and validation

---

## Architecture

### Network Stack Enhancement

```
┌─────────────────────────────────────────────────────────────┐
│                    HandControl Server                       │
├─────────────────────────────────────────────────────────────┤
│  Network Interface Enumeration                              │
│  ┌───────────────────────────────────────────────────────┐  │
│  │ 1. Enumerate all network interfaces (if-addrs crate)  │  │
│  │ 2. Filter unwanted interfaces (Docker, VPN, etc.)     │  │
│  │ 3. Collect all IPv4 and IPv6 addresses               │  │
│  │ 4. Apply IPv6 filtering rules                        │  │
│  │ 5. Prioritize addresses                               │  │
│  └───────────────────────────────────────────────────────┘  │
├─────────────────────────────────────────────────────────────┤
│  IPv6 Address Filtering                                     │
│  ┌───────────────────────────────────────────────────────┐  │
│  │ - Exclude link-local (fe80::/10) UNLESS same subnet  │  │
│  │ - Exclude deprecated addresses                        │  │
│  │ - Prefer stable over temporary (privacy extensions)   │  │
│  │ - Exclude ULA (fc00::/7) if public addresses exist   │  │
│  └───────────────────────────────────────────────────────┘  │
├─────────────────────────────────────────────────────────────┤
│  Address Prioritization Algorithm                           │
│  ┌───────────────────────────────────────────────────────┐  │
│  │ Priority Order:                                        │  │
│  │ 1. Global IPv6 (2000::/3)          [Score: 100]      │  │
│  │ 2. Public IPv4                      [Score: 80]       │  │
│  │ 3. ULA IPv6 (fc00::/7)              [Score: 60]       │  │
│  │ 4. Private IPv4 (RFC1918)           [Score: 40]       │  │
│  │ 5. Link-local IPv6 (fe80::/10)      [Score: 20]       │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│                    Android Client                           │
├─────────────────────────────────────────────────────────────┤
│  Connection Strategy (Happy Eyeballs RFC 8305)              │
│  ┌───────────────────────────────────────────────────────┐  │
│  │ 1. Receive multiple IPs from server (IPv4 + IPv6)    │  │
│  │ 2. Sort by priority (prefer IPv6)                     │  │
│  │ 3. Try primary address first                          │  │
│  │ 4. If no response after 250ms, try next address      │  │
│  │ 5. Continue parallel attempts                         │  │
│  │ 6. First successful connection wins                   │  │
│  └───────────────────────────────────────────────────────┘  │
├─────────────────────────────────────────────────────────────┤
│  IPv6 Network Detection                                     │
│  ┌───────────────────────────────────────────────────────┐  │
│  │ - Detect if device has IPv6 connectivity             │  │
│  │ - Check for IPv6-only network (464XLAT detection)    │  │
│  │ - Adjust connection strategy based on capabilities    │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

---

## Implementation Plan

### Phase 1: Server Multi-IP with IPv6 Filtering

**Prerequisite:** Complete Multi-IP Enrollment (MULTI_IP_ENROLLMENT.md)

#### 1.1 Network Interface Enumeration

**File:** `src/network/interfaces.rs` (NEW)

```rust
use if_addrs::{get_if_addrs, IfAddr, Interface};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub struct NetworkInterface {
    pub name: String,
    pub addr: IpAddr,
    pub is_loopback: bool,
    pub is_up: bool,
    pub priority: u8,
}

pub fn get_all_network_addresses() -> Result<Vec<NetworkInterface>> {
    let interfaces = get_if_addrs()?;

    let filtered: Vec<NetworkInterface> = interfaces
        .into_iter()
        .filter(|iface| should_include_interface(iface))
        .map(|iface| NetworkInterface::from_if_addr(iface))
        .collect();

    Ok(filtered)
}

fn should_include_interface(iface: &Interface) -> bool {
    // Exclude loopback
    if iface.is_loopback() {
        return false;
    }

    // Exclude common virtual interfaces
    let excluded_prefixes = ["docker", "veth", "br-", "virbr", "tun", "tap"];
    if excluded_prefixes.iter().any(|prefix| iface.name.starts_with(prefix)) {
        return false;
    }

    // Include only IPv4 and IPv6 addresses
    matches!(iface.addr, IfAddr::V4(_) | IfAddr::V6(_))
}
```

#### 1.2 IPv6 Address Classification

**File:** `src/network/ipv6.rs` (NEW)

```rust
use std::net::Ipv6Addr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ipv6AddressScope {
    LinkLocal,      // fe80::/10
    UniqueLocal,    // fc00::/7 (ULA)
    Global,         // 2000::/3
    Multicast,      // ff00::/8
    Other,
}

impl Ipv6AddressScope {
    pub fn from_address(addr: &Ipv6Addr) -> Self {
        let segments = addr.segments();

        // Link-local: fe80::/10
        if (segments[0] & 0xffc0) == 0xfe80 {
            return Self::LinkLocal;
        }

        // Unique Local Address: fc00::/7
        if (segments[0] & 0xfe00) == 0xfc00 {
            return Self::UniqueLocal;
        }

        // Global unicast: 2000::/3
        if (segments[0] & 0xe000) == 0x2000 {
            return Self::Global;
        }

        // Multicast: ff00::/8
        if segments[0] == 0xff00 {
            return Self::Multicast;
        }

        Self::Other
    }
}

pub fn is_ipv6_temporary(addr: &Ipv6Addr) -> bool {
    // Temporary addresses (privacy extensions RFC 4941) typically have
    // random interface identifiers. This is a heuristic check.
    // For more accurate detection, we'd need OS-specific APIs.

    // Check if interface identifier looks random (not EUI-64)
    let segments = addr.segments();
    let iid = [segments[4], segments[5], segments[6], segments[7]];

    // EUI-64 has 0xfffe in the middle (segments[5])
    // If not present, likely temporary
    iid[1] != 0xfffe
}

pub fn should_use_ipv6_address(addr: &Ipv6Addr, has_global: bool) -> bool {
    let scope = Ipv6AddressScope::from_address(addr);

    match scope {
        // Always exclude multicast and other
        Ipv6AddressScope::Multicast | Ipv6AddressScope::Other => false,

        // Include link-local only if no global addresses available
        Ipv6AddressScope::LinkLocal => !has_global,

        // Include ULA if no global available
        Ipv6AddressScope::UniqueLocal => !has_global,

        // Always include global
        Ipv6AddressScope::Global => true,
    }
}
```

#### 1.3 Address Prioritization

**File:** `src/network/priority.rs` (NEW)

```rust
use std::net::IpAddr;
use crate::network::ipv6::{Ipv6AddressScope, is_ipv6_temporary};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AddressPriority {
    score: u8,
    is_temporary: bool,  // Lower priority if temporary
}

impl AddressPriority {
    pub fn calculate(addr: &IpAddr) -> Self {
        let (score, is_temporary) = match addr {
            IpAddr::V4(ipv4) => {
                if is_public_ipv4(ipv4) {
                    (80, false)
                } else {
                    (40, false)  // Private IPv4
                }
            }
            IpAddr::V6(ipv6) => {
                let scope = Ipv6AddressScope::from_address(ipv6);
                let is_temp = is_ipv6_temporary(ipv6);

                let base_score = match scope {
                    Ipv6AddressScope::Global => 100,
                    Ipv6AddressScope::UniqueLocal => 60,
                    Ipv6AddressScope::LinkLocal => 20,
                    _ => 0,
                };

                (base_score, is_temp)
            }
        };

        Self { score, is_temporary }
    }
}

fn is_public_ipv4(addr: &std::net::Ipv4Addr) -> bool {
    !addr.is_private()
        && !addr.is_loopback()
        && !addr.is_link_local()
        && !addr.is_broadcast()
        && !addr.is_documentation()
}

pub fn sort_addresses_by_priority(addrs: &mut Vec<IpAddr>) {
    addrs.sort_by_key(|addr| {
        let priority = AddressPriority::calculate(addr);
        // Reverse order for stable addresses (prefer stable over temporary)
        (std::cmp::Reverse(priority.score), priority.is_temporary)
    });
}
```

#### 1.4 Configuration

**File:** `src/config/mod.rs` (UPDATE)

```toml
# config.toml additions
[network]
# IPv6 configuration
prefer_ipv6 = true                    # Prefer IPv6 addresses in listings
include_link_local = false            # Include fe80:: addresses
include_ula = true                    # Include fc00:: addresses (ULA)
prefer_stable_addresses = true        # Prefer stable over temporary IPv6

# Interface filtering
excluded_interface_prefixes = ["docker", "veth", "br-", "virbr", "tun", "tap"]

# Address limits
max_advertised_addresses = 5          # Max IPs to include in QR/mDNS
```

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NetworkConfig {
    #[serde(default = "default_prefer_ipv6")]
    pub prefer_ipv6: bool,

    #[serde(default)]
    pub include_link_local: bool,

    #[serde(default = "default_true")]
    pub include_ula: bool,

    #[serde(default = "default_true")]
    pub prefer_stable_addresses: bool,

    #[serde(default = "default_excluded_interfaces")]
    pub excluded_interface_prefixes: Vec<String>,

    #[serde(default = "default_max_addresses")]
    pub max_advertised_addresses: usize,
}

fn default_prefer_ipv6() -> bool { true }
fn default_true() -> bool { true }
fn default_max_addresses() -> usize { 5 }
fn default_excluded_interfaces() -> Vec<String> {
    vec![
        "docker".to_string(),
        "veth".to_string(),
        "br-".to_string(),
        "virbr".to_string(),
        "tun".to_string(),
        "tap".to_string(),
    ]
}
```

#### 1.5 Update Server Info

**File:** `src/grpc/server.rs` (UPDATE)

Replace `get_local_ip()` with new multi-IP function:

```rust
use crate::network::{get_all_network_addresses, sort_addresses_by_priority};

pub fn get_prioritized_addresses(config: &NetworkConfig) -> Result<Vec<IpAddr>> {
    let interfaces = get_all_network_addresses()?;

    let mut addresses: Vec<IpAddr> = interfaces
        .into_iter()
        .map(|iface| iface.addr)
        .filter(|addr| should_include_address(addr, config))
        .collect();

    sort_addresses_by_priority(&mut addresses);

    // Limit to configured max
    addresses.truncate(config.max_advertised_addresses);

    Ok(addresses)
}

fn should_include_address(addr: &IpAddr, config: &NetworkConfig) -> bool {
    match addr {
        IpAddr::V4(_) => true,
        IpAddr::V6(ipv6) => {
            let scope = Ipv6AddressScope::from_address(ipv6);

            match scope {
                Ipv6AddressScope::Global => true,
                Ipv6AddressScope::UniqueLocal => config.include_ula,
                Ipv6AddressScope::LinkLocal => config.include_link_local,
                _ => false,
            }
        }
    }
}
```

---

### Phase 2: Protocol Updates

#### 2.1 Update Proto Definitions

**File:** `proto/handcontrol.proto` (UPDATE)

```protobuf
message GenerateEnrollmentQRResponse {
  bool success = 1;
  string qr_data = 2;              // JSON payload
  string error_message = 3;

  // Structured data for display
  string server_id = 4;
  repeated string ips = 5;          // NEW: Multiple IPs (IPv4 + IPv6)
  int32 port = 6;
  string cert_fingerprint = 7;
  string enrollment_token = 8;

  // Deprecated (kept for backward compatibility)
  string ip = 9 [deprecated = true];
}

message ServerInfoResponse {
  string server_id = 1;
  string hostname = 2;
  string version = 3;
  string os = 4;
  repeated string local_ips = 5;   // NEW: All local IPs for discovery
}
```

#### 2.2 QR Code Payload Format

**File:** `src/security/enrollment.rs` (UPDATE)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentQrPayload {
    pub server_id: String,
    pub ips: Vec<String>,           // NEW: Multiple IPs
    pub port: u16,
    pub cert_fingerprint: String,
    pub enrollment_token: String,

    // Deprecated but kept for backward compatibility
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
}

impl EnrollmentQrPayload {
    pub fn new(
        server_id: String,
        addresses: Vec<IpAddr>,
        port: u16,
        cert_fingerprint: String,
        enrollment_token: String,
    ) -> Self {
        let ips: Vec<String> = addresses.iter().map(|a| a.to_string()).collect();

        // Include first IP in deprecated field for backward compatibility
        let ip = addresses.first().map(|a| a.to_string());

        Self {
            server_id,
            ips,
            port,
            cert_fingerprint,
            enrollment_token,
            ip,
        }
    }
}
```

---

### Phase 3: Android IPv6 Support

#### 3.1 Connection Manager Update

**File:** `app/src/main/kotlin/com/handcontrol/core/network/ConnectionManager.kt` (UPDATE)

```kotlin
class SmartConnectionManager(
    private val channelFactory: MtlsGrpcChannelFactory
) {
    /**
     * Implements Happy Eyeballs algorithm (RFC 8305) for connection establishment.
     * Tries IPv6 first, falls back to IPv4 if needed.
     */
    suspend fun connect(server: EnrolledServer): ManagedChannel {
        val addresses = server.ips.sortedByDescending {
            calculateAddressPriority(it)
        }

        if (addresses.isEmpty()) {
            throw ConnectionException("No addresses available")
        }

        return connectWithHappyEyeballs(
            addresses = addresses,
            port = server.port,
            tlsConfig = server.toTlsConfig()
        )
    }

    private suspend fun connectWithHappyEyeballs(
        addresses: List<String>,
        port: Int,
        tlsConfig: TlsConfig
    ): ManagedChannel {
        val primaryAddress = addresses.first()

        // Try primary address first
        val primaryDeferred = async {
            tryConnect(primaryAddress, port, tlsConfig)
        }

        // Wait 250ms for primary connection (RFC 8305 recommendation)
        delay(250)

        if (primaryDeferred.isCompleted) {
            val result = primaryDeferred.await()
            if (result.isSuccess) {
                return result.getOrThrow()
            }
        }

        // Start parallel attempts for remaining addresses
        val fallbackDeferreds = addresses.drop(1).map { address ->
            async {
                tryConnect(address, port, tlsConfig)
            }
        }

        // Wait for first successful connection
        val allAttempts = listOf(primaryDeferred) + fallbackDeferreds

        return selectFirstSuccessful(allAttempts)
            ?: throw ConnectionException("All connection attempts failed")
    }

    private suspend fun tryConnect(
        address: String,
        port: Int,
        tlsConfig: TlsConfig
    ): Result<ManagedChannel> = runCatching {
        channelFactory.createChannel(
            host = address,
            port = port,
            tlsConfig = tlsConfig
        )
    }

    private fun calculateAddressPriority(address: String): Int {
        val ip = try {
            InetAddress.getByName(address)
        } catch (e: Exception) {
            return 0
        }

        return when {
            // Global IPv6
            ip is Inet6Address && isGlobalIpv6(ip) -> 100

            // Public IPv4
            ip is Inet4Address && !ip.isSiteLocalAddress -> 80

            // ULA IPv6
            ip is Inet6Address && isUlaIpv6(ip) -> 60

            // Private IPv4
            ip is Inet4Address && ip.isSiteLocalAddress -> 40

            // Link-local IPv6
            ip is Inet6Address && ip.isLinkLocalAddress -> 20

            else -> 0
        }
    }

    private fun isGlobalIpv6(addr: Inet6Address): Boolean {
        val bytes = addr.address
        // Check if in 2000::/3 range
        return (bytes[0].toInt() and 0xe0) == 0x20
    }

    private fun isUlaIpv6(addr: Inet6Address): Boolean {
        val bytes = addr.address
        // Check if in fc00::/7 range
        return (bytes[0].toInt() and 0xfe) == 0xfc
    }

    private suspend fun selectFirstSuccessful(
        attempts: List<Deferred<Result<ManagedChannel>>>
    ): ManagedChannel? {
        return supervisorScope {
            try {
                select<ManagedChannel?> {
                    attempts.forEach { deferred ->
                        deferred.onAwait { result ->
                            result.getOrNull()
                        }
                    }
                }
            } finally {
                // Cancel remaining attempts
                attempts.forEach { it.cancel() }
            }
        }
    }
}
```

#### 3.2 Database Schema Update

**File:** `app/src/main/kotlin/com/handcontrol/core/data/local/EnrolledServer.kt` (UPDATE)

```kotlin
@Entity(tableName = "enrolled_servers")
data class EnrolledServer(
    @PrimaryKey val serverId: String,
    val serverName: String,
    val ips: List<String>,          // NEW: Multiple IPs (was single 'host')
    val port: Int,
    val clientId: String,
    val certFingerprint: String,
    val enrolledAt: Long,
    val lastConnectedAt: Long? = null,

    // Deprecated but kept for migration
    @Deprecated("Use ips instead")
    val host: String? = null
)

// Type converters for Room
class Converters {
    @TypeConverter
    fun fromStringList(value: List<String>): String {
        return value.joinToString(",")
    }

    @TypeConverter
    fun toStringList(value: String): List<String> {
        return if (value.isEmpty()) emptyList() else value.split(",")
    }
}
```

#### 3.3 Migration

**File:** `app/src/main/kotlin/com/handcontrol/core/data/local/AppDatabase.kt` (UPDATE)

```kotlin
val MIGRATION_1_2 = object : Migration(1, 2) {
    override fun migrate(database: SupportSQLiteDatabase) {
        // Add ips column
        database.execSQL(
            "ALTER TABLE enrolled_servers ADD COLUMN ips TEXT NOT NULL DEFAULT ''"
        )

        // Migrate existing host values to ips
        database.execSQL(
            "UPDATE enrolled_servers SET ips = host WHERE host IS NOT NULL"
        )
    }
}
```

#### 3.4 QR Scanner Update

**File:** `app/src/main/kotlin/com/handcontrol/feature/enrollment/QrScannerViewModel.kt` (UPDATE)

```kotlin
fun parseQrCode(qrData: String): Result<EnrollmentData> = runCatching {
    val json = JSONObject(qrData)

    val serverId = json.getString("server_id")
    val port = json.getInt("port")
    val certFingerprint = json.getString("cert_fingerprint")
    val enrollmentToken = json.getString("enrollment_token")

    // Parse multiple IPs (new format)
    val ips = if (json.has("ips")) {
        val ipsArray = json.getJSONArray("ips")
        List(ipsArray.length()) { ipsArray.getString(it) }
    } else if (json.has("ip")) {
        // Backward compatibility with old format
        listOf(json.getString("ip"))
    } else {
        throw IllegalArgumentException("No IP addresses in QR code")
    }

    EnrollmentData(
        serverId = serverId,
        ips = ips,
        port = port,
        certFingerprint = certFingerprint,
        enrollmentToken = enrollmentToken
    )
}
```

---

### Phase 4: mDNS IPv6 Support

#### 4.1 Server mDNS Advertisement

**File:** `src/mdns/service.rs` (UPDATE)

```rust
pub fn advertise_service(
    config: &ServerConfig,
    addresses: Vec<IpAddr>,
    cert_fingerprint: &str,
) -> Result<MdnsService> {
    let mdns = ServiceDaemon::new()?;

    let service_type = "_handcontrol._tcp.local.";
    let instance_name = config.mdns_instance_name.clone()
        .unwrap_or_else(|| hostname::get()?.to_string_lossy().to_string());

    // Separate IPv4 and IPv6 addresses
    let ipv4_addrs: Vec<Ipv4Addr> = addresses.iter()
        .filter_map(|a| if let IpAddr::V4(v4) = a { Some(*v4) } else { None })
        .collect();

    let ipv6_addrs: Vec<Ipv6Addr> = addresses.iter()
        .filter_map(|a| if let IpAddr::V6(v6) = a { Some(*v6) } else { None })
        .collect();

    // Create TXT records
    let mut txt_properties = HashMap::new();
    txt_properties.insert("version".to_string(), "1.0".to_string());
    txt_properties.insert("server_id".to_string(), config.server_id.to_string());
    txt_properties.insert("cert_fingerprint".to_string(), cert_fingerprint.to_string());
    txt_properties.insert("ipv6_supported".to_string(), (!ipv6_addrs.is_empty()).to_string());

    // Register service with both IPv4 and IPv6 addresses
    let service_info = ServiceInfo::new(
        service_type,
        &instance_name,
        &format!("{}.local.", instance_name),
        &ipv4_addrs[..],
        config.server.port,
        Some(txt_properties),
    )?
    .with_ipv6_addrs(&ipv6_addrs[..]);  // Add IPv6 addresses

    let fullname = mdns.register(service_info)?;

    tracing::info!(
        "mDNS service registered: {} with {} IPv4 and {} IPv6 addresses",
        fullname,
        ipv4_addrs.len(),
        ipv6_addrs.len()
    );

    Ok(MdnsService { mdns, fullname })
}
```

#### 4.2 Android mDNS Discovery

**File:** `app/src/main/kotlin/com/handcontrol/core/discovery/AndroidNsdDiscoveryManager.kt` (UPDATE)

```kotlin
override fun onServiceResolved(serviceInfo: NsdServiceInfo) {
    val serverId = serviceInfo.attributes["server_id"]?.decodeToString()
    val certFingerprint = serviceInfo.attributes["cert_fingerprint"]?.decodeToString()
    val ipv6Supported = serviceInfo.attributes["ipv6_supported"]?.decodeToString()?.toBoolean() ?: false

    // Collect all addresses (IPv4 + IPv6)
    val addresses = mutableListOf<String>()

    // Add primary host (IPv4 or IPv6)
    serviceInfo.host?.hostAddress?.let { addresses.add(it) }

    // Add additional addresses if available (Android 12+)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        serviceInfo.hostAddresses.forEach { inetAddress ->
            addresses.add(inetAddress.hostAddress ?: return@forEach)
        }
    }

    val discoveredServer = DiscoveredServer(
        serverId = serverId ?: "unknown",
        serverName = serviceInfo.serviceName,
        ips = addresses.distinct(),  // Multiple IPs
        port = serviceInfo.port,
        certFingerprint = certFingerprint,
        ipv6Supported = ipv6Supported
    )

    _discoveredServers.update { servers ->
        servers + discoveredServer
    }
}
```

---

### Phase 5: Testing & Validation

#### 5.1 Unit Tests

**File:** `src/network/tests.rs` (NEW)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipv6_scope_detection() {
        // Global
        let global = "2001:db8::1".parse::<Ipv6Addr>().unwrap();
        assert_eq!(Ipv6AddressScope::from_address(&global), Ipv6AddressScope::Global);

        // Link-local
        let link_local = "fe80::1".parse::<Ipv6Addr>().unwrap();
        assert_eq!(Ipv6AddressScope::from_address(&link_local), Ipv6AddressScope::LinkLocal);

        // ULA
        let ula = "fc00::1".parse::<Ipv6Addr>().unwrap();
        assert_eq!(Ipv6AddressScope::from_address(&ula), Ipv6AddressScope::UniqueLocal);
    }

    #[test]
    fn test_address_prioritization() {
        let mut addrs = vec![
            "192.168.1.10".parse().unwrap(),      // Private IPv4
            "2001:db8::1".parse().unwrap(),       // Global IPv6
            "fe80::1".parse().unwrap(),           // Link-local IPv6
            "203.0.113.1".parse().unwrap(),       // Public IPv4
            "fc00::1".parse().unwrap(),           // ULA IPv6
        ];

        sort_addresses_by_priority(&mut addrs);

        // Expected order: Global IPv6, Public IPv4, ULA, Private IPv4, Link-local
        assert!(matches!(addrs[0], IpAddr::V6(_)));
        assert!(matches!(addrs[1], IpAddr::V4(_)));
        assert_eq!(addrs[0].to_string(), "2001:db8::1");
        assert_eq!(addrs[1].to_string(), "203.0.113.1");
    }
}
```

#### 5.2 Integration Tests

**File:** `tests/ipv6_connectivity_test.rs` (NEW)

```rust
#[tokio::test]
async fn test_dual_stack_server_binding() {
    // Test that server can bind to :: and accept both IPv4 and IPv6
}

#[tokio::test]
async fn test_ipv6_enrollment_flow() {
    // Test QR enrollment with IPv6 address
}

#[tokio::test]
async fn test_multi_ip_connection_fallback() {
    // Test that client tries multiple IPs if first fails
}
```

#### 5.3 Android Tests

**File:** `app/src/androidTest/.../ConnectionManagerTest.kt` (NEW)

```kotlin
@Test
fun testHappyEyeballsAlgorithm() {
    // Mock multiple addresses
    // Verify IPv6 tried first
    // Verify fallback to IPv4
    // Verify parallel connection attempts
}

@Test
fun testIpv6AddressParsing() {
    // Test QR code with IPv6 addresses
    // Test mDNS discovery with IPv6
}
```

---

### Phase 6: Documentation

#### 6.1 User Documentation

**File:** `docs/USER_GUIDE_IPV6.md` (NEW)

- How to verify IPv6 connectivity
- Expected behavior on IPv6-only networks
- Troubleshooting IPv6 connection issues
- How to prefer IPv4 if needed

#### 6.2 Admin Documentation

**File:** `docs/ADMIN_IPV6_DEPLOYMENT.md` (NEW)

- Configuring server for IPv6
- Firewall rules for IPv6
- Router configuration tips
- IPv6 address planning
- Common deployment scenarios

#### 6.3 Architecture Documentation

**File:** `docs/ARCHITECTURE_IPV6.md` (NEW)

- IPv6 address selection algorithm
- Happy Eyeballs implementation details
- mDNS IPv6 advertisement
- Security considerations with IPv6

---

## Success Criteria

### Functional Requirements

- [ ] Server enumerates all IPv4 and IPv6 addresses
- [ ] IPv6 addresses properly filtered (exclude link-local unless needed)
- [ ] Addresses prioritized (prefer IPv6 global > public IPv4)
- [ ] QR codes contain multiple IPs (IPv4 + IPv6)
- [ ] mDNS advertises both IPv4 and IPv6 addresses
- [ ] Android connects via IPv6 when available
- [ ] Android falls back to IPv4 if IPv6 fails
- [ ] Happy Eyeballs algorithm implemented (250ms delay)
- [ ] Configuration options for IPv6 behavior

### Performance Requirements

- [ ] Connection establishment < 500ms on IPv6
- [ ] Fallback to IPv4 < 1 second total
- [ ] No performance degradation vs IPv4-only
- [ ] Minimal battery impact from multi-address attempts

### Compatibility Requirements

- [ ] Backward compatible with IPv4-only networks
- [ ] Works on dual-stack networks
- [ ] Works on IPv6-only networks (with NAT64)
- [ ] Old Android clients can still connect (single IP fallback)
- [ ] Old servers work with new Android clients

---

## Testing Strategy

### Network Scenarios

1. **IPv4-only network**: Both client and server on IPv4
2. **Dual-stack network**: Both have IPv4 and IPv6
3. **IPv6-only client**: Mobile network with NAT64
4. **IPv6-only server**: Server with only IPv6 address
5. **Mixed environment**: Client IPv4, server dual-stack
6. **Multiple subnets**: Server on multiple network interfaces

### Test Matrix

| Client Network | Server Network | Expected Behavior |
|----------------|----------------|-------------------|
| IPv4 only      | IPv4 only      | Connect via IPv4 |
| IPv4 only      | Dual-stack     | Connect via IPv4 |
| IPv6 only      | IPv6 only      | Connect via IPv6 |
| IPv6 only      | Dual-stack     | Connect via IPv6 |
| Dual-stack     | Dual-stack     | Prefer IPv6, fallback IPv4 |
| Dual-stack     | IPv4 only      | Connect via IPv4 |
| Dual-stack     | IPv6 only      | Connect via IPv6 |

---

## Dependencies

### New Crate Dependencies

```toml
[dependencies]
if-addrs = "0.13"    # Network interface enumeration
```

### Android Dependencies

No new dependencies required (standard Java networking APIs).

---

## Migration Path

### Server Migration

1. Update server binary to version with IPv6 support
2. Server automatically enumerates IPv6 addresses
3. Configuration file updated with `[network]` section (optional)
4. Old QR codes still work (single IP for backward compatibility)
5. New QR codes include multiple IPs

### Android Migration

1. Update Android app to version with multi-IP support
2. App auto-migrates database (host -> ips)
3. Old enrollments continue to work
4. New enrollments leverage IPv6
5. Connection strategy automatically optimized

### Rollback Plan

If issues arise:
- Server: Set `prefer_ipv6 = false` in config
- Server: Set `max_advertised_addresses = 1` to limit to single IP
- Android: Falls back to first IP in list (backward compatible)

---

## Timeline

### Week 1: Server Foundation
- Implement network interface enumeration
- Implement IPv6 filtering and prioritization
- Add configuration options
- Unit tests

### Week 2: Protocol & Integration
- Update proto definitions
- Update QR code format
- Update mDNS advertisement
- Integration tests

### Week 3: Android Implementation
- Implement Happy Eyeballs algorithm
- Update database schema and migration
- Update QR scanner
- Update mDNS discovery

### Week 4: Testing & Documentation
- End-to-end testing on various networks
- Performance testing
- Write user and admin documentation
- Bug fixes

**Total Estimated Time:** 4 weeks (20 working days)

---

## Risk Assessment

### Technical Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| IPv6 not available on user networks | High | Low | Fallback to IPv4 works seamlessly |
| Performance regression | Low | Medium | Comprehensive benchmarking before release |
| mDNS IPv6 issues | Medium | Medium | Extensive testing on different routers |
| Android IPv6 bugs | Low | High | Test on multiple Android versions |
| Link-local addressing issues | Medium | Low | Exclude by default, document use case |

### Operational Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| User confusion about IPv6 | Medium | Low | Clear documentation, automatic behavior |
| Firewall blocking IPv6 | High | Medium | Fallback to IPv4, document firewall rules |
| ISP IPv6 issues | Medium | Low | Fallback to IPv4 works automatically |

---

## Future Enhancements (Post-v1)

These features are explicitly out of scope for this phase but may be considered later:

- **IPv6-only operation** (no IPv4 fallback)
- **NAT64/DNS64 detection and optimization**
- **IPv6 transition mechanism support** (6to4, Teredo)
- **IPv6 prefix delegation** for complex network setups
- **Dynamic IPv6 address change detection** (router renumbering)
- **IPv6 multicast for discovery** (instead of mDNS)

---

## Approval & Sign-off

This feature plan requires approval before implementation begins.

**Prepared by:** Claude Code Assistant
**Date:** 2025-10-30
**Status:** Pending Review

---

## References

- RFC 8305: Happy Eyeballs Version 2 (Dual Stack)
- RFC 4941: Privacy Extensions for IPv6
- RFC 4007: IPv6 Scoped Address Architecture
- RFC 3484: Default Address Selection for IPv6
- MULTI_IP_ENROLLMENT.md: Multi-IP enrollment specification
- PRD.md: HandControl Product Requirements Document
- SECURITY.md: HandControl Security Architecture
