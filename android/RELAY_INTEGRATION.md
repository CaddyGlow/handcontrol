# Android Relay Integration

This document describes the relay integration implementation for the HandControl Android client (Phase 3).

## Overview

The Android client now supports automatic fallback from direct mTLS connections to relay tunneling when servers are not directly reachable (e.g., on different networks, behind NAT, firewalled).

## Architecture

### Core Components

1. **RelayTunnelFactory** (`com.handcontrol.core.network.relay.RelayTunnelFactory`)
   - Opens WebSocket connections to relay server
   - Implements handshake protocol: ConnectMessage → ConnectAck → TunnelReadyMessage
   - Provides bidirectional byte channels for gRPC traffic
   - Returns `RelayTunnel` with `incomingData` channel and `sendData()` method

2. **RelayGrpcChannelFactory** (`com.handcontrol.core.network.relay.RelayGrpcChannelFactory`)
   - Creates gRPC channels over relay tunnels
   - Starts local TCP bridge (ServerSocket on random port)
   - Forwards socket data ↔ WebSocket tunnel bidirectionally
   - Preserves end-to-end mTLS encryption

3. **ServerConnectionManager** (`com.handcontrol.core.network.ServerConnectionManager`)
   - Smart connection strategy with automatic fallback
   - Tries direct connection to each server IP (5s timeout)
   - Falls back to relay if direct fails (10s timeout)
   - Returns `ConnectionResult` with channel, mode, and address

### Data Flow

```
┌─────────────┐     mTLS/gRPC     ┌──────────────┐     WebSocket     ┌──────────────┐
│   Android   │ ← → Local Bridge  │ Relay Tunnel │ ← → Binary Frames │ Relay Server │
│  gRPC Stub  │     (localhost)   │   Factory    │    (encrypted)    │              │
└─────────────┘                   └──────────────┘                   └──────────────┘
                                                                             ↓
                                                              ┌──────────────────────┐
                                                              │   Target Server      │
                                                              │  (mTLS gRPC Server)  │
                                                              └──────────────────────┘
```

### Connection Strategy

1. **Direct Connection Attempt**:
   - Try each server IP address sequentially
   - 5 second timeout per IP
   - If any IP succeeds, use direct connection

2. **Relay Fallback**:
   - Triggered when all direct attempts fail
   - Checks if `relayEnabled && relayUrl && relayToken` are present
   - Connects to relay via WebSocket with JWT authentication
   - 10 second timeout for relay connection

3. **Connection Result**:
   ```kotlin
   data class ConnectionResult(
       val channel: ManagedChannel,
       val mode: ConnectionMode,  // DIRECT or RELAY
       val connectedAddress: String? = null
   )
   ```

## Database Schema

Relay fields in `EnrolledServerEntity`:

```kotlin
@Entity(tableName = "enrolled_servers")
data class EnrolledServerEntity(
    // ... existing fields ...

    val relayEnabled: Boolean = false,
    val relayUrl: String? = null,
    val relayToken: String? = null,
    val lastConnectionMode: ConnectionMode = ConnectionMode.UNKNOWN
)
```

## Enrollment Flow

During enrollment (both QR and Approval modes), the server may include relay information:

```protobuf
message EnrollResponse {
    bool success = 1;
    string client_id = 2;
    optional RelayInfo relay_info = 4;
}

message RelayInfo {
    string relay_url = 1;
    string relay_token = 2;
    bool relay_required = 3;
}
```

The `GrpcEnrollmentRepository` parses this and saves to database:

```kotlin
val relayEnabled = response.hasRelayInfo() && !response.relayInfo.relayUrl.isEmpty()
val relayUrl = if (relayEnabled) response.relayInfo.relayUrl else null
val relayToken = if (relayEnabled) response.relayInfo.relayToken else null
```

## UI Indicators

### Server List Screen

- **Relay Available**: Shows Cloud icon + "Relay" label in secondary color
- **Direct Only**: Shows WiFi icon (dimmed)

### Server Details Screen

- **Relay Support**: "Available" or "Not configured"
- **Relay Server**: Displays relay URL (if configured)
- **Last Connection Mode**: "Direct", "Relay", or "Unknown"

## Testing

### Manual Testing Scenarios

1. **Same Network (Direct)**:
   - Enroll server on same WiFi
   - Connect → should use direct mode
   - Verify WiFi icon in server list

2. **Different Network (Relay Fallback)**:
   - Switch Android to mobile data
   - Connect → should try direct, fail, then use relay
   - Verify Cloud icon + "Relay" label
   - Check server details shows "Relay" as last connection mode

3. **No Relay Available**:
   - Enroll server without relay configured
   - Switch to different network
   - Connect → should fail with error

### Unit Testing

Create tests for:

- `RelayTunnelFactory`: WebSocket handshake, binary data forwarding
- `ServerConnectionManager`: Fallback logic, timeout handling
- `EnrolledServerRepository`: Relay field persistence

### Integration Testing

Test end-to-end flow:

1. Start relay server (see `relay/relay.toml`)
2. Start handcontrol server with relay enabled
3. Enroll Android client
4. Verify relay info saved
5. Connect from different network
6. Verify relay connection works

## Security Considerations

1. **End-to-End mTLS**: Relay server only sees encrypted TLS bytes, cannot decrypt traffic
2. **JWT Authentication**: Relay token authenticates client to relay server
3. **Token Expiry**: Tokens have 24-hour TTL (configurable), clients must re-enroll after expiry
4. **Certificate Pinning**: Server certificate still validated end-to-end

## Known Limitations

1. **No Token Refresh**: Clients must re-enroll when relay tokens expire (currently 24 hours)
2. **No Relay Certificate Validation**: Relay server certificate is accepted without validation
3. **No User Preferences**: Cannot force relay or disable relay (always tries direct first)

## Future Work

### Phase 3 Remaining

1. **Integration**:
   - Update `GrpcCommandRepository` to use `ServerConnectionManager`
   - Update `ServerHealthCheckerImpl` to use `ServerConnectionManager`

2. **User Settings**:
   - Show connection statistics (direct vs relay usage)

3. **Testing**:
   - Write unit tests for relay components
   - Create mock relay server for integration tests
   - Add instrumentation tests for relay fallback

### Phase 4

1. **Documentation**:
   - User guide for relay feature
   - Troubleshooting common relay issues
   - Administrator guide for relay server setup

2. **Performance**:
   - Measure relay latency overhead
   - Optimize tunnel buffer sizes
   - Add connection pooling/reuse

3. **Reliability**:
   - Implement token refresh mechanism
   - Add relay server certificate validation
   - Handle relay disconnections gracefully

## Files Modified

### New Files

- `android/app/src/main/kotlin/com/handcontrol/core/network/relay/RelayTunnelFactory.kt`
- `android/app/src/main/kotlin/com/handcontrol/core/network/relay/RelayGrpcChannelFactory.kt`
- `android/app/src/main/kotlin/com/handcontrol/core/network/ServerConnectionManager.kt`

### Modified Files

- `android/app/build.gradle.kts`: Added OkHttp and kotlinx-serialization dependencies
- `android/app/src/main/kotlin/com/handcontrol/data/database/EnrolledServerRepository.kt`: Added relay parameters to `saveServer()`
- `android/app/src/main/kotlin/com/handcontrol/data/enrollment/GrpcEnrollmentRepository.kt`: Parse and save relay info from enrollment responses
- `android/app/src/main/kotlin/com/handcontrol/core/model/ServerHealthStatus.kt`: Added relay fields to `ServerDetailInfo`
- `android/app/src/main/kotlin/com/handcontrol/feature/serverlist/ServerListScreen.kt`: Added relay indicators to server cards
- `android/app/src/main/kotlin/com/handcontrol/feature/serverdetails/ServerDetailsScreen.kt`: Added relay information display
- `android/app/src/main/kotlin/com/handcontrol/feature/serverdetails/ServerDetailsViewModel.kt`: Map relay fields to ServerDetailInfo

## Configuration

No user-facing configuration required. Relay is automatically enabled when server provides relay information during enrollment.

## Troubleshooting

### Relay Connection Fails

1. Check server logs for relay client connection
2. Verify relay token is not expired
3. Check relay server is reachable from Android device
4. Verify server registered with relay successfully

### Always Uses Direct

1. Verify server has relay configured (`relay.enabled = true`)
2. Check enrollment response includes `relay_info`
3. Verify relay fields saved in database

### Cannot Connect At All

1. Check both direct and relay connection attempts in logs
2. Verify network connectivity
3. Check certificate validation passes
4. Verify server is running and accessible

## References

- [FEATURE_RELAY.md](../docs/FEATURE_RELAY.md): Complete relay feature specification
- [PROGRESS.md](../relay/PROGRESS.md): Implementation progress tracking
- [handcontrol.proto](../proto/handcontrol.proto): Protocol definitions including RelayInfo
