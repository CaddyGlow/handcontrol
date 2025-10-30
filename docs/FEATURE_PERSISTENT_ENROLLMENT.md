# Feature Specification: Persistent Enrollment & Auto-Reconnection

## 1. Overview

### Problem Statement
After enrolling an Android device with a PC and closing the application, users must re-enroll each time they reopen the app. The app successfully stores client credentials (client_id, certificate) but does not persist the server connection information (host, port, server_id), making reconnection impossible.

### Solution Summary
Implement persistent storage for enrolled server information using Room database, enabling automatic reconnection to previously enrolled servers when the app launches.

### Scope
- Android client only (server-side already handles reconnection correctly via mTLS)
- Multi-server support (users can enroll with multiple PCs)
- Auto-reconnect to last used server on app launch
- Future extensibility for server list UI and manual server selection
- No backward compatibility requirements (feature ships before first public release)

---

## 2. Current State Analysis

### What Works
- **Server-side persistence**: Server stores enrolled clients in `~/.config/handcontrol/authorized_clients/` with metadata
- **Client credential storage**: Android stores `client_id` in DataStore and certificate in Android Keystore
- **mTLS validation**: Server validates returning clients via certificate authentication
- **Certificate pinning (TOFU)**: Android stores `server_fingerprint` for trust-on-first-use validation

### What's Missing
- **Server connection details**: host, port, server_id, server_name not persisted
- **Navigation state loss**: App forgets which server to connect to after closing
- **Multi-server management**: Can only track one server_fingerprint in current DataStore implementation
- **Client credential mapping**: Single `client_id` value in DataStore gets overwritten when enrolling with multiple servers
- **Reconnection logic**: No startup check for enrolled servers

### Data Flow Gap
```
Current Flow:
1. Enroll → Store client_id + certificate + server_fingerprint
2. Navigate to Commands (host/port in navigation params)
3. Close app → Navigation params lost
4. Reopen app → No stored server info → Must re-enroll

Needed Flow:
1. Enroll → Store client_id + certificate + server info (host/port/id)
2. Navigate to Commands
3. Close app
4. Reopen app → Load last server info → Auto-navigate to Commands
```

---

## 3. Requirements

### Functional Requirements

#### FR1: Persistent Server Storage
- System SHALL store enrolled server information locally on Android device
- Storage SHALL include: server_id, server_host, server_port, client_id, server_name, cert_fingerprint, enrolled_at, last_connected
- Storage SHALL support multiple enrolled servers
- Storage SHALL survive app restarts and device reboots

#### FR2: Enrollment Enhancement
- After successful enrollment (QR or approval mode), system SHALL:
  1. Call `GetServerInfo` RPC to retrieve server metadata
  2. Store complete server information in local database
  3. Mark server as "last connected"

#### FR3: Auto-Reconnection
- On app launch, system SHALL:
  1. Check if any enrolled servers exist
  2. If yes, load the last connected server
  3. Auto-navigate to CommandList screen with saved host/port
  4. If no, show Welcome screen with "Get Started" button
- After each successful connection (auto or manual), system SHALL refresh the `last_connected` timestamp for the active server.

#### FR4: Multi-Server Support
- System SHALL allow enrollment with multiple PCs
- System SHALL track separate client_id for each server
- System SHALL store separate cert_fingerprint for each server
- System SHALL track last_connected timestamp for each server

#### FR5: Connection Validation
- Before auto-reconnecting, system SHOULD verify server is reachable (optional graceful degradation)
- If connection fails, system SHOULD show error and return to Welcome screen

#### FR6: Data Integrity
- System SHALL persist each `client_id` together with its `server_id` in Room as the single source of truth
- System SHALL remove legacy single-value DataStore usage for server credentials
- System SHALL initialize storage from scratch on upgrade (no backward-compatibility guarantees required)

### Non-Functional Requirements

#### NFR1: Performance
- Database queries SHALL complete in <100ms
- Auto-reconnection SHALL complete in <2 seconds from app launch

#### NFR2: Security
- Server credentials (certificates, private keys) SHALL remain in Android Keystore
- Database SHALL NOT store private key material
- Fingerprints SHALL be stored securely alongside server information

#### NFR3: Reliability
- System SHALL handle database corruption gracefully
- System SHALL handle missing server scenarios (deleted from server-side)
- System SHALL handle network failures during reconnection

#### NFR4: Maintainability
- Database schema SHALL be versioned with Room migrations
- Repository pattern SHALL abstract database implementation

---

## 4. Technical Design

### 4.1 Data Model

#### EnrolledServerEntity (Room Entity)
```kotlin
@Entity(tableName = "enrolled_servers")
data class EnrolledServerEntity(
    @PrimaryKey
    val serverId: String,           // UUID from GetServerInfo RPC

    val serverHost: String,         // IPv4/IPv6 address
    val serverPort: Int,            // Port number (default 50051)
    val clientId: String,           // UUID assigned by server during enrollment
    val serverName: String,         // Hostname from GetServerInfo
    val certFingerprint: String,    // SHA256 fingerprint for TOFU

    @ColumnInfo(name = "enrolled_at")
    val enrolledAt: Long,           // Unix timestamp (milliseconds)

    @ColumnInfo(name = "last_connected")
    val lastConnected: Long?        // Unix timestamp, null if never connected after enrollment
)
```

#### Database Indices
```kotlin
@Entity(
    tableName = "enrolled_servers",
    indices = [
        Index(value = ["last_connected"], name = "idx_last_connected"),
        Index(value = ["server_host", "server_port"], name = "idx_host_port")
    ]
)
```

### 4.2 Database Schema

#### HandControlDatabase
```kotlin
@Database(
    entities = [EnrolledServerEntity::class],
    version = 1,
    exportSchema = true
)
abstract class HandControlDatabase : RoomDatabase() {
    abstract fun enrolledServerDao(): EnrolledServerDao
}
```

#### DAO Interface
```kotlin
@Dao
interface EnrolledServerDao {
    @Query("SELECT * FROM enrolled_servers ORDER BY (last_connected IS NULL), last_connected DESC")
    fun getAllServers(): Flow<List<EnrolledServerEntity>>

    @Query("SELECT * FROM enrolled_servers WHERE serverId = :serverId")
    suspend fun getServerById(serverId: String): EnrolledServerEntity?

    @Query("SELECT * FROM enrolled_servers ORDER BY (last_connected IS NULL), last_connected DESC LIMIT 1")
    suspend fun getLastConnectedServer(): EnrolledServerEntity?

    @Query("SELECT * FROM enrolled_servers ORDER BY (last_connected IS NULL), last_connected DESC LIMIT 1")
    fun observeLastConnectedServer(): Flow<EnrolledServerEntity?>

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insertServer(server: EnrolledServerEntity)

    @Query("UPDATE enrolled_servers SET last_connected = :timestamp WHERE serverId = :serverId")
    suspend fun updateLastConnected(serverId: String, timestamp: Long)

    @Delete
    suspend fun deleteServer(server: EnrolledServerEntity)

    @Query("DELETE FROM enrolled_servers WHERE serverId = :serverId")
    suspend fun deleteServerById(serverId: String)
}
```

### 4.3 Repository Layer

#### EnrolledServerRepository
```kotlin
class EnrolledServerRepository(
    private val enrolledServerDao: EnrolledServerDao
) {
    val allServers: Flow<List<EnrolledServerEntity>> =
        enrolledServerDao.getAllServers()

    val lastConnectedServer: Flow<EnrolledServerEntity?> =
        enrolledServerDao.observeLastConnectedServer()

    suspend fun saveServer(
        serverId: String,
        serverHost: String,
        serverPort: Int,
        clientId: String,
        serverName: String,
        certFingerprint: String
    ) {
        val server = EnrolledServerEntity(
            serverId = serverId,
            serverHost = serverHost,
            serverPort = serverPort,
            clientId = clientId,
            serverName = serverName,
            certFingerprint = certFingerprint,
            enrolledAt = System.currentTimeMillis(),
            lastConnected = System.currentTimeMillis()
        )
        enrolledServerDao.insertServer(server)
    }

    suspend fun updateLastConnected(serverId: String) {
        enrolledServerDao.updateLastConnected(serverId, System.currentTimeMillis())
    }

    suspend fun removeServer(serverId: String) {
        enrolledServerDao.deleteServerById(serverId)
    }
}
```

`lastConnectedServer` exposes a cold `Flow` that the UI can collect to react to changes without blocking the main thread.

### 4.4 Integration Points

#### 4.4.1 Enrollment Flow Enhancement

**File: `GrpcEnrollmentRepository.kt`**

**QR Code Enrollment (enrollWithToken method):**
```kotlin
// After line 73 (if response.success)
if (response.success) {
    // NEW: Fetch server info and save to database
    val serverInfo = getServerInfo(channel)
    enrolledServerRepository.saveServer(
        serverId = serverInfo.serverId,
        serverHost = host,
        serverPort = port,
        clientId = response.clientId,
        serverName = serverInfo.hostname,
        certFingerprint = extractedFingerprint
    )
}
```

**Approval Enrollment (pollApprovalStatus method):**
```kotlin
// After line 220 (when status is APPROVED)
PairingStatus.APPROVED -> {
    val clientId = response.clientId
    val serverInfo = getServerInfo(channel)
    enrolledServerRepository.saveServer(
        serverId = serverInfo.serverId,
        serverHost = host,
        serverPort = port,
        clientId = clientId,
        serverName = serverInfo.hostname,
        certFingerprint = extractedFingerprint
    )
}
```

**New method to add:**
```kotlin
private suspend fun getServerInfo(channel: ManagedChannel): ServerInfoResponse {
    val stub = HandControlServiceGrpcKt.HandControlServiceCoroutineStub(channel)
    return stub.getServerInfo(ServerInfoRequest.getDefaultInstance())
}
```

#### 4.4.2 Auto-Reconnection Logic

**File: `WelcomeViewModel.kt`**

Expose observable state for auto-reconnect:
```kotlin
private val _autoReconnectServer: StateFlow<EnrolledServerEntity?> =
    enrolledServerRepository.lastConnectedServer
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

val autoReconnectServer: StateFlow<EnrolledServerEntity?> = _autoReconnectServer

init {
    viewModelScope.launch {
        autoReconnectServer.collect { lastServer ->
            _uiState.update {
                it.copy(
                    hasEnrolledServer = lastServer != null,
                    lastServerName = lastServer?.serverName,
                    autoReconnect = lastServer != null
                )
            }
        }
    }
}

fun onServerConnected(serverId: String) {
    viewModelScope.launch {
        enrolledServerRepository.updateLastConnected(serverId)
    }
}
```

**File: `HandControlNavHost.kt`**

Modify start destination logic:
```kotlin
@Composable
fun HandControlNavHost(
    modifier: Modifier = Modifier,
    welcomeViewModel: WelcomeViewModel = hiltViewModel()
) {
    val navController = rememberNavController()

    val autoReconnectServer by welcomeViewModel.autoReconnectServer
        .collectAsStateWithLifecycle(initialValue = null)

    NavHost(
        navController = navController,
        startDestination = Route.Welcome,
        modifier = modifier
    ) {
        // ... rest of navigation graph
    }

    LaunchedEffect(autoReconnectServer?.serverId) {
        val server = autoReconnectServer ?: return@LaunchedEffect
        navController.navigate(
            Route.CommandList(server.serverHost, server.serverPort)
        ) {
            popUpTo(Route.Welcome) { inclusive = true }
        }
    }
}
```

> `collectAsStateWithLifecycle` comes from `androidx.lifecycle:lifecycle-runtime-compose`.

When `CommandList` confirms a successful connection (initial load or manual switch), it should invoke `welcomeViewModel.onServerConnected(serverId)` so `last_connected` reflects actual usage.

#### 4.4.3 Certificate Manager Enhancement

**File: `AndroidKeystoreCertificateManager.kt`**

Current storage uses single `server_fingerprint` key. Update to keep fingerprints exclusively in Room:
- Remove `server_fingerprint` from DataStore entirely
- Store fingerprint in `EnrolledServerEntity.certFingerprint`
- Update validation logic to read fingerprints from `EnrolledServerRepository`

### 4.5 Initialization Strategy

- On first launch, initialize an empty Room database and remove any legacy DataStore keys (`client_id`, `server_fingerprint`) if present.
- When enrolling for the first time after upgrade, treat the flow as a clean slate (re-enrollment required if prior data is lost).
- No backward-compatibility or migration logic is required because the product has not shipped yet.

---

## 5. User Experience Flow

### 5.1 First-Time Enrollment
```
1. User opens app → Welcome screen
2. User taps "Get Started"
3. User scans QR code or enters pairing code
4. App enrolls successfully
5. App calls GetServerInfo RPC
6. App saves server info to Room database
7. App navigates to CommandList screen
8. After CommandList confirms connectivity, app updates `last_connected` for this server
```

### 5.2 Subsequent App Launches (Auto-Reconnect)
```
1. User opens app
2. App queries Room for last connected server
3. If found:
   a. App auto-navigates to CommandList with saved host/port
   b. CommandList establishes mTLS connection (validates certificate)
   c. CommandList notifies ViewModel to refresh `last_connected`
   d. User sees command list immediately
4. If not found:
   a. App shows Welcome screen
   b. User taps "Get Started" to enroll
```

### 5.3 Multi-Server Scenario (Future Enhancement)
```
1. User enrolled with Server A, closes app
2. User opens app → Auto-connects to Server A
3. User wants to enroll with Server B
4. User navigates to "Add Server" (new UI)
5. User enrolls with Server B
6. App saves Server B info, marks as last connected
7. Next app launch → Auto-connects to Server B
8. User can access "Server List" screen to switch between A and B
```

---

## 6. Security Considerations

### 6.1 Data Protection
- Server credentials remain in Android Keystore (hardware-backed, never exported)
- Room database stores only non-sensitive metadata (host, port, IDs, timestamps)
- Fingerprints stored in database are public values (SHA256 of server cert)
- Database file inherits Android app sandbox protections

### 6.2 Trust Model
- TOFU (Trust On First Use) maintained: first enrollment captures server fingerprint
- Subsequent connections validate against stored fingerprint
- Multi-server support requires separate fingerprint per server_id

### 6.3 Threat Mitigation
- **Server impersonation**: mTLS + fingerprint validation prevents MITM
- **Database tampering**: Android sandbox + file permissions protect database
- **Credential theft**: Private keys never leave Keystore, database has no secrets
- **Denial of service**: Auto-reconnect includes timeout and error handling

### 6.4 Privacy
- No telemetry or external data transmission beyond existing mTLS connections
- Server list stored locally only
- No cloud sync or backup (respects user privacy)

---

## 7. Testing Strategy

### 7.1 Unit Tests
- `EnrolledServerDao` CRUD operations
- `EnrolledServerRepository` business logic
- Initialization path creates clean database state when no servers exist

### 7.2 Integration Tests
- Enrollment flow saves to Room correctly
- Auto-reconnect reads from Room correctly
- Multi-server scenarios (enroll A, enroll B, switch between)

### 7.3 UI Tests
- Welcome screen shows auto-reconnect when server exists
- Navigation flow from Welcome → Commands (auto-reconnect)
- Error handling when server unreachable

### 7.4 Manual Testing Scenarios
1. **Fresh install → Enroll → Close app → Reopen** (should auto-reconnect)
2. **Enroll → Server offline → Reopen app** (should show error, return to Welcome)
3. **Enroll with Server A → Enroll with Server B → Reopen app** (should connect to B)
4. **Enroll → Uninstall server → Reopen app** (should handle gracefully)

---

## 8. Implementation Checklist

### Phase 1: Database Setup
- [ ] Add Room dependencies to `android/app/build.gradle.kts`
- [ ] Create `EnrolledServerEntity.kt`
- [ ] Create `EnrolledServerDao.kt`
- [ ] Create `HandControlDatabase.kt`
- [ ] Create `EnrolledServerRepository.kt`
- [ ] Add Hilt dependency injection for database

### Phase 2: Enrollment Enhancement
- [ ] Modify `GrpcEnrollmentRepository.kt` - add `getServerInfo()` method
- [ ] Update `enrollWithToken()` to persist server info (including `client_id`) to Room after success
- [ ] Update `pollApprovalStatus()` to persist server info when APPROVED
- [ ] Remove legacy DataStore writes so Room owns `client_id` and fingerprint persistence
- [ ] Add error handling for GetServerInfo RPC failures

### Phase 3: Auto-Reconnection
- [ ] Update `WelcomeViewModel.kt` - expose Flow/StateFlow of last connected server and hook `onServerConnected`
- [ ] Modify `HandControlNavHost.kt` - collect Flow and navigate without blocking the main thread
- [ ] Invoke `welcomeViewModel.onServerConnected(serverId)` after each successful connection/selection
- [ ] Add connection validation before auto-reconnect (optional)
- [ ] Handle auto-reconnect failures gracefully (navigate back + surface error)

### Phase 4: Certificate Manager Update
- [ ] Remove DataStore fingerprint storage
- [ ] Update validation logic to read fingerprints from `EnrolledServerRepository`
- [ ] Confirm certificate pinning continues to work with Room-only storage

### Phase 5: Testing
- [ ] Write unit tests for DAO and repository
- [ ] Write integration tests for enrollment + storage
- [ ] Write UI tests for auto-reconnect flow
- [ ] Manual testing with real server

### Phase 6: Documentation
- [ ] Update `docs/PRD.md` with persistent enrollment feature
- [ ] Update `docs/PROJECT_STRUCTURE.md` with new database components
- [ ] Document Room schema and initialization behavior for developers

---

## 9. Future Enhancements

### 9.1 Server List UI
- Add "Enrolled Servers" screen showing all saved servers
- Allow manual server selection
- Show last connected timestamp
- Show server status (online/offline)

### 9.2 Server Management
- "Forget Server" / unenroll action
- Edit server nickname
- Sort servers by name/last connected

### 9.3 Connection Health
- Periodic server reachability checks
- Show connection status indicator
- Auto-switch to another enrolled server if current is unreachable

### 9.4 Backup & Sync
- Export/import enrolled servers (encrypted)
- Cloud sync for multiple Android devices (privacy-respecting)

---

## 10. Dependencies

### New Android Dependencies
```kotlin
// build.gradle.kts (app module)
dependencies {
    // Room
    implementation("androidx.room:room-runtime:2.6.1")
    implementation("androidx.room:room-ktx:2.6.1")
    ksp("androidx.room:room-compiler:2.6.1")
}
```

### Server-Side Dependencies
None required (server already handles reconnection via mTLS)

---

## 11. Risks & Mitigations

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Database corruption | High | Low | Regular backups, schema tests, error handling |
| Residual legacy preferences | Low | Medium | Remove obsolete DataStore keys during initialization |
| Server unreachable on auto-reconnect | Medium | Medium | Timeout + error handling, return to Welcome screen |
| Certificate/fingerprint mismatch | High | Low | Strict validation, clear error messages |
| Multiple devices with same certificate | Medium | Low | Server-side tracks last_seen, Android generates unique certs |

---

## 12. Success Criteria

### Definition of Done
- [ ] Users can close and reopen app without re-enrolling
- [ ] Auto-reconnection works within 2 seconds of app launch
- [ ] Multi-server enrollment supported (database stores multiple servers)
- [ ] All existing enrollment flows (QR + approval) save server info
- [ ] Unit tests pass with >80% coverage
- [ ] Integration tests pass for enrollment + reconnection
- [ ] Manual testing confirms no regressions

### Metrics
- **Time to reconnect**: <2 seconds from app launch to command list
- **Enrollment retention**: 100% (no need to re-enroll after app close)
- **Database query performance**: <100ms for all queries
- **Test coverage**: >80% for new database and repository code

---

## 13. References

### Related Documents
- `docs/PRD.md` - Product Requirements Document (multi-server support mentioned)
- `docs/SECURITY.md` - Security requirements (mTLS, TOFU, logging)
- `docs/PROJECT_STRUCTURE.md` - Codebase organization

### Code References
- `android/app/src/main/kotlin/com/handcontrol/data/enrollment/GrpcEnrollmentRepository.kt` - Enrollment logic
- `android/app/src/main/kotlin/com/handcontrol/core/security/AndroidKeystoreCertificateManager.kt` - Certificate storage
- `proto/handcontrol.proto` - GetServerInfo RPC definition
- `src/storage/clients.rs` - Server-side client persistence

---

**Document Version:** 1.0
**Last Updated:** 2025-10-30
**Author:** Claude Code Agent
**Status:** Draft - Ready for Implementation
