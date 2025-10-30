# Feature Specification: Config Hot-Reload with Client Notifications

**Version:** 1.0
**Date:** 2025-10-30
**Status:** Draft
**Related Documents:** PRD.md, SECURITY.md, PROJECT_STRUCTURE.md

---

## Overview

### Goals

Enable the HandControl server to automatically reload its configuration file when changes are detected, and notify connected Android clients so they can refresh their command lists without manual intervention.

**Primary Objectives:**
- Eliminate need for server restart when modifying commands
- Provide seamless user experience when adding/editing/removing commands
- Maintain system stability and security during config changes
- Support both connected and disconnected client scenarios

**Benefits:**
- Faster iteration when configuring commands
- No service interruption for config updates
- Better user experience (commands appear automatically on Android)
- Reduced operational complexity

### Scope

**In Scope:**
- Automatic file watching for `config.toml` changes
- Hot-reload of command definitions (add/remove/modify)
- Real-time notification to connected clients via gRPC streaming
- Polling fallback for clients without active stream
- Config version tracking
- Validation and rollback on invalid config
- Android client auto-refresh on notification

**Out of Scope (Require Server Restart):**
- Network settings (`port`, `bind_address`)
- Security/enrollment settings (`cert_path`, `key_path`, enrollment modes)
- mDNS configuration changes
- Certificate regeneration

**Rationale for Scope Limitation:**
Commands are the most frequently modified config elements and can be safely hot-reloaded without affecting core server infrastructure. Security and network changes require careful coordination and are better handled via restart.

---

## Current State Analysis

### Existing Architecture

**Config Loading:**
- Single load at startup in `src/main.rs:start_handcontrol_server()`
- Immutable `Arc<Config>` shared across gRPC service handlers
- No file watching or reload mechanism

**Command Serving:**
- `ListCommands` RPC converts `config.command[]` to protobuf on-demand
- Commands are always current snapshot from `Arc<Config>`
- No caching or versioning

**Client Behavior:**
- Android fetches command list on screen load
- No automatic refresh mechanism
- User must navigate away and back to refresh

### Gaps Addressed by This Feature

1. **No hot-reload capability** - Config changes require full server restart
2. **No client notification** - Clients unaware of config changes
3. **Manual refresh burden** - Users must manually refresh command list
4. **No version tracking** - Cannot detect if client has stale commands

---

## Architecture

### Component Overview

```
┌─────────────────────────────────────────────────────────┐
│                    Server Components                     │
├─────────────────────────────────────────────────────────┤
│                                                           │
│  ┌──────────────────┐      ┌─────────────────────────┐ │
│  │  File Watcher    │──────>│  Config Reloader        │ │
│  │  (notify crate)  │      │  (RwLock<Config>)       │ │
│  └──────────────────┘      └──────────┬──────────────┘ │
│                                        │                 │
│                                        │ trigger         │
│                                        v                 │
│  ┌─────────────────────────────────────────────────────┐│
│  │         Config Change Broadcaster                    ││
│  │  - Increment config version                          ││
│  │  - Broadcast to all active stream subscribers        ││
│  │  - Store latest version for pollers                  ││
│  └──────────────────┬───────────────────────────────────┘│
│                     │                                     │
└─────────────────────┼─────────────────────────────────────┘
                      │ gRPC
                      │ (streaming + polling)
┌─────────────────────┼─────────────────────────────────────┐
│                     v              Android Client          │
├─────────────────────────────────────────────────────────┤
│                                                           │
│  ┌──────────────────────────────────────────────────┐   │
│  │  WatchConfigUpdates Stream                       │   │
│  │  (long-lived gRPC stream, primary notification)  │   │
│  └─────────────────┬────────────────────────────────┘   │
│                    │                                      │
│                    │ on disconnect/failure                │
│                    v                                      │
│  ┌──────────────────────────────────────────────────┐   │
│  │  Polling Fallback                                │   │
│  │  - GetConfigVersion RPC every 30s                │   │
│  │  - Compare version with local cache              │   │
│  └─────────────────┬────────────────────────────────┘   │
│                    │                                      │
│                    │ version changed                      │
│                    v                                      │
│  ┌──────────────────────────────────────────────────┐   │
│  │  Command List Auto-Refresh                       │   │
│  │  - Re-fetch ListCommands RPC                     │   │
│  │  - Update ViewModel state                        │   │
│  │  - UI automatically reflects new commands        │   │
│  └──────────────────────────────────────────────────┘   │
│                                                           │
└───────────────────────────────────────────────────────────┘
```

### Data Flow

**1. Config File Modified (by user editing config.toml):**
```
User saves config.toml
  -> File watcher detects change (notify crate)
  -> Config Reloader triggered
  -> Parse and validate new config
  -> If valid: Swap RwLock<Config> with new config
  -> If invalid: Log error, keep old config
  -> Increment config_version (AtomicU64)
  -> Broadcast ConfigUpdateNotification to all active streams
```

**2. Connected Client Receives Update (streaming path):**
```
Server broadcasts ConfigUpdateNotification
  -> Client receives via WatchConfigUpdates stream
  -> ConfigRepository processes notification
  -> CommandListViewModel.refreshCommands() called
  -> ListCommands RPC fetched
  -> UI updated with new command list
  -> User sees new commands instantly
```

**3. Disconnected Client Polls (fallback path):**
```
Client polling timer fires (every 30s)
  -> GetConfigVersion RPC called
  -> Server returns current_version
  -> Client compares with local cached_version
  -> If different: Trigger refreshCommands()
  -> Update cached_version
```

**4. New Client Connects:**
```
Client connects to server
  -> Subscribes to WatchConfigUpdates stream
  -> Fetches initial ListCommands
  -> Stores config_version from response
  -> Stream remains open for future updates
```

---

## Server Implementation

### 1. Config State Management

**Change from immutable Arc to mutable RwLock:**

**Before:**
```rust
// src/main.rs
let config = Arc::new(load_config(&config_path)?);
let service = RemoteControlService::new(config.clone());
```

**After:**
```rust
// src/main.rs
let config = Arc::new(RwLock::new(load_config(&config_path)?));
let config_version = Arc::new(AtomicU64::new(1));
let service = RemoteControlService::new(
    config.clone(),
    config_version.clone(),
    config_broadcaster.clone(),
);
```

**Rationale:**
- `RwLock` allows multiple concurrent readers (command execution) with exclusive writer (reload)
- `AtomicU64` version counter prevents ABA problem and enables client version comparison
- `Arc` enables sharing across threads (file watcher, gRPC handlers, broadcaster)

### 2. File Watching

**Implementation: `src/config/watcher.rs` (new file)**

```rust
use notify::{Watcher, RecursiveMode, Result as NotifyResult};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::mpsc;

pub struct ConfigWatcher {
    config_path: PathBuf,
    config: Arc<RwLock<Config>>,
    config_version: Arc<AtomicU64>,
    broadcaster: Arc<ConfigBroadcaster>,
}

impl ConfigWatcher {
    pub async fn start(self) -> Result<()> {
        let (tx, mut rx) = mpsc::channel(100);

        // Debounce file system events (editors often write multiple times)
        let mut watcher = notify::recommended_watcher(move |res| {
            if let Ok(event) = res {
                let _ = tx.blocking_send(event);
            }
        })?;

        watcher.watch(&self.config_path, RecursiveMode::NonRecursive)?;

        // Event processing loop
        let mut debounce_timer = tokio::time::interval(Duration::from_millis(500));
        let mut pending_reload = false;

        loop {
            tokio::select! {
                Some(event) = rx.recv() => {
                    if matches!(event.kind, EventKind::Modify(_)) {
                        pending_reload = true;
                    }
                }
                _ = debounce_timer.tick() => {
                    if pending_reload {
                        self.reload_config().await?;
                        pending_reload = false;
                    }
                }
            }
        }
    }

    async fn reload_config(&self) -> Result<()> {
        // Parse new config
        let new_config = match load_config(&self.config_path) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::error!("Config reload failed: {}, keeping old config", e);
                return Ok(()); // Don't propagate error, keep running with old config
            }
        };

        // Validate commands only changed (enforce scope)
        let old_config = self.config.read().unwrap();
        if !config_reload_allowed(&old_config, &new_config) {
            tracing::warn!("Config contains non-command changes, restart required");
            return Ok(());
        }
        drop(old_config);

        // Atomic swap
        {
            let mut config_guard = self.config.write().unwrap();
            *config_guard = new_config;
        }

        // Increment version and broadcast
        let new_version = self.config_version.fetch_add(1, Ordering::SeqCst) + 1;
        tracing::info!("Config reloaded successfully, version: {}", new_version);

        self.broadcaster.broadcast(ConfigUpdateNotification {
            version: new_version,
            timestamp_ms: current_timestamp_ms(),
        }).await;

        Ok(())
    }
}

fn config_reload_allowed(old: &Config, new: &Config) -> bool {
    // Only allow command changes, reject other modifications
    old.server == new.server && old.security == new.security
}
```

**Key Design Decisions:**
- **Debouncing (500ms):** Prevents multiple reloads when editors save multiple times
- **Error handling:** Invalid config logs error but keeps old config (availability over consistency)
- **Validation:** Rejects changes to server/security settings, logs warning
- **Logging:** Uses `tracing` for structured observability

### 3. Config Broadcasting

**Implementation: `src/config/broadcaster.rs` (new file)**

```rust
use tokio::sync::broadcast;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct ConfigUpdateNotification {
    pub version: u64,
    pub timestamp_ms: i64,
}

pub struct ConfigBroadcaster {
    tx: broadcast::Sender<ConfigUpdateNotification>,
}

impl ConfigBroadcaster {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub async fn broadcast(&self, notification: ConfigUpdateNotification) {
        // Ignore send errors (no active subscribers is fine)
        let _ = self.tx.send(notification);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ConfigUpdateNotification> {
        self.tx.subscribe()
    }
}
```

**Rationale:**
- `tokio::sync::broadcast` provides multi-consumer pub/sub
- Bounded channel (capacity 100) prevents memory growth if clients lag
- Dropping oldest messages on overflow is acceptable (clients will poll)

### 4. New gRPC RPCs

**Add to `proto/handcontrol.proto`:**

```protobuf
service RemoteControl {
  // ... existing RPCs ...

  // Get current config version for polling clients
  rpc GetConfigVersion(GetConfigVersionRequest) returns (GetConfigVersionResponse);

  // Subscribe to config update notifications (server-streaming)
  rpc WatchConfigUpdates(WatchConfigUpdatesRequest) returns (stream ConfigUpdateNotification);
}

// Get config version (for polling)
message GetConfigVersionRequest {}

message GetConfigVersionResponse {
  uint64 config_version = 1;       // Current config version number
  int64 last_updated_ms = 2;       // Unix timestamp of last config reload
}

// Watch config updates (streaming)
message WatchConfigUpdatesRequest {}

message ConfigUpdateNotification {
  uint64 config_version = 1;       // New config version number
  int64 timestamp_ms = 2;          // Unix timestamp of this update
}

// Add config_version to ListCommandsResponse
message ListCommandsResponse {
  repeated Command commands = 1;
  uint64 config_version = 2;       // Version of config used for this list
}
```

**Implementation: `src/grpc/config.rs` (new file)**

```rust
impl RemoteControl for RemoteControlService {
    async fn get_config_version(
        &self,
        _request: Request<GetConfigVersionRequest>,
    ) -> Result<Response<GetConfigVersionResponse>, Status> {
        let version = self.config_version.load(Ordering::SeqCst);
        let last_updated = self.last_config_update.load(Ordering::SeqCst);

        Ok(Response::new(GetConfigVersionResponse {
            config_version: version,
            last_updated_ms: last_updated,
        }))
    }

    async fn watch_config_updates(
        &self,
        _request: Request<WatchConfigUpdatesRequest>,
    ) -> Result<Response<Self::WatchConfigUpdatesStream>, Status> {
        let mut rx = self.broadcaster.subscribe();

        let stream = async_stream::stream! {
            while let Ok(notification) = rx.recv().await {
                yield Ok(notification);
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }
}
```

**Update existing ListCommands to include version:**

```rust
async fn list_commands(
    &self,
    _request: Request<ListCommandsRequest>,
) -> Result<Response<ListCommandsResponse>, Status> {
    let config = self.config.read().unwrap();
    let version = self.config_version.load(Ordering::SeqCst);

    let commands = config.command.iter()
        .map(|cmd| /* convert to protobuf */)
        .collect();

    Ok(Response::new(ListCommandsResponse {
        commands,
        config_version: version,
    }))
}
```

### 5. Integration in main.rs

**Updated startup flow:**

```rust
#[tokio::main]
async fn main() -> Result<()> {
    // ... logging setup ...

    let config_path = get_config_path()?;
    let config = Arc::new(RwLock::new(load_config(&config_path)?));
    let config_version = Arc::new(AtomicU64::new(1));
    let broadcaster = Arc::new(ConfigBroadcaster::new(100));

    // Start config watcher task
    let watcher = ConfigWatcher::new(
        config_path.clone(),
        config.clone(),
        config_version.clone(),
        broadcaster.clone(),
    );
    tokio::spawn(async move {
        if let Err(e) = watcher.start().await {
            tracing::error!("Config watcher failed: {}", e);
        }
    });

    // Start gRPC server with reloadable config
    let service = RemoteControlService::new(
        config.clone(),
        config_version.clone(),
        broadcaster.clone(),
    );

    // ... rest of server setup ...
}
```

---

## Client Implementation (Android)

### 1. Repository Layer Updates

**`GrpcCommandRepository.kt` additions:**

```kotlin
class GrpcCommandRepository : CommandRepository {
    private val configUpdateFlow = MutableSharedFlow<ConfigUpdate>(replay = 1)
    private var cachedConfigVersion: ULong = 0UL
    private var watchJob: Job? = null
    private var pollingJob: Job? = null

    override fun watchConfigUpdates(): Flow<ConfigUpdate> = configUpdateFlow

    suspend fun startWatchingConfig(host: String, port: Int) {
        // Primary: streaming subscription
        watchJob?.cancel()
        watchJob = coroutineScope.launch {
            try {
                val channel = createChannel(host, port)
                val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)

                stub.watchConfigUpdates(WatchConfigUpdatesRequest.getDefaultInstance())
                    .catch { e ->
                        tracing.warn("Config watch stream error: $e, falling back to polling")
                        startPollingFallback(host, port)
                    }
                    .collect { notification ->
                        handleConfigUpdate(notification.configVersion)
                    }
            } catch (e: Exception) {
                tracing.error("Failed to start config watch: $e")
                startPollingFallback(host, port)
            }
        }
    }

    private fun startPollingFallback(host: String, port: Int) {
        pollingJob?.cancel()
        pollingJob = coroutineScope.launch {
            while (isActive) {
                delay(30_000) // Poll every 30 seconds
                try {
                    val channel = createChannel(host, port)
                    val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)

                    val response = stub.getConfigVersion(
                        GetConfigVersionRequest.getDefaultInstance()
                    )

                    if (response.configVersion > cachedConfigVersion) {
                        handleConfigUpdate(response.configVersion)
                    }
                } catch (e: Exception) {
                    tracing.debug("Polling error: $e")
                }
            }
        }
    }

    private suspend fun handleConfigUpdate(newVersion: ULong) {
        cachedConfigVersion = newVersion
        configUpdateFlow.emit(ConfigUpdate(version = newVersion))
    }

    override suspend fun listCommands(host: String, port: Int): List<Command> {
        val response = stub.listCommands(ListCommandsRequest.getDefaultInstance())
        cachedConfigVersion = response.configVersion
        return response.commandsList.map { /* convert from protobuf */ }
    }
}

data class ConfigUpdate(val version: ULong)
```

### 2. ViewModel Layer

**`CommandListViewModel.kt` updates:**

```kotlin
class CommandListViewModel(
    private val repository: CommandRepository
) : ViewModel() {

    private val _commands = MutableStateFlow<List<Command>>(emptyList())
    val commands: StateFlow<List<Command>> = _commands.asStateFlow()

    init {
        // Watch for config updates and auto-refresh
        viewModelScope.launch {
            repository.watchConfigUpdates()
                .collect { update ->
                    tracing.info("Config updated to version ${update.version}, refreshing")
                    refreshCommands()
                }
        }
    }

    fun connectToServer(host: String, port: Int) {
        viewModelScope.launch {
            // Initial load
            loadCommands(host, port)

            // Start watching for updates
            repository.startWatchingConfig(host, port)
        }
    }

    private suspend fun refreshCommands() {
        try {
            val newCommands = repository.listCommands(currentHost, currentPort)
            _commands.value = newCommands
            tracing.info("Commands refreshed, count: ${newCommands.size}")
        } catch (e: Exception) {
            tracing.error("Failed to refresh commands: $e")
        }
    }
}
```

### 3. UI Layer (No Changes Required)

**Key benefit:** UI automatically updates via StateFlow observation in Compose:

```kotlin
@Composable
fun CommandListScreen(viewModel: CommandListViewModel) {
    val commands by viewModel.commands.collectAsState()

    LazyColumn {
        items(commands) { command ->
            CommandCard(command = command)
        }
    }
    // When commands StateFlow updates, Compose automatically recomposes
}
```

---

## Security Considerations

### Authentication and Authorization

**Access Control:**
- `WatchConfigUpdates` - **mTLS required** (enrolled clients only)
- `GetConfigVersion` - **mTLS required** (enrolled clients only)
- No unauthenticated access to config metadata

**Rationale:** Config version leaks information about server activity patterns. Require authentication for all config-related RPCs.

### Config Validation

**Safety Checks on Reload:**

1. **Parse validation:** Reject syntactically invalid TOML
2. **Semantic validation:** Reject commands with invalid parameter types
3. **Scope enforcement:** Reject changes to server/security settings
4. **Rollback on failure:** Keep old config if new one is invalid

**Implementation:**
```rust
fn validate_config_reload(old: &Config, new: &Config) -> Result<(), ConfigError> {
    // 1. Only commands changed
    if old.server != new.server || old.security != new.security {
        return Err(ConfigError::NonCommandChanges);
    }

    // 2. All commands have valid IDs
    for cmd in &new.command {
        if cmd.id.is_empty() {
            return Err(ConfigError::InvalidCommandId);
        }
    }

    // 3. No duplicate command IDs
    let ids: HashSet<_> = new.command.iter().map(|c| &c.id).collect();
    if ids.len() != new.command.len() {
        return Err(ConfigError::DuplicateCommandId);
    }

    Ok(())
}
```

### Race Condition Protection

**Scenario: Client executes command during config reload**

```
Thread 1 (client): Read command list
Thread 2 (watcher): Reload config (delete command)
Thread 1 (client): Execute now-deleted command
```

**Mitigation:** RwLock semantics prevent this race:
1. Client acquires read lock for ListCommands
2. Watcher blocks on write lock until all readers finish
3. Client finishes reading, releases lock
4. Watcher acquires write lock, swaps config
5. If client later executes, gets NOT_FOUND error (safe)

**Additional safety:** ExecuteCommand validates command exists at execution time, not list time.

### Logging and Auditing

**Events to Log:**

```rust
tracing::info!("Config reload triggered by file change");
tracing::info!("Config reloaded successfully, version: {}", version);
tracing::warn!("Config validation failed: {}, keeping old config", error);
tracing::error!("Config watcher error: {}", error);
```

**Do NOT log:**
- Full config contents (may contain sensitive shell commands)
- Command parameters (could leak secrets)
- Exact file paths (security through obscurity)

---

## Error Handling

### Server-Side Error Scenarios

**1. Invalid Config File**
```
File modified -> Parse fails
  -> Log error with line number
  -> Keep old config (availability)
  -> Do NOT broadcast update
  -> Server continues running with old config
```

**2. Non-Command Config Changes**
```
File modified -> Validation detects server.port changed
  -> Log warning: "Restart required for non-command changes"
  -> Keep old config
  -> Do NOT broadcast update
```

**3. File System Watcher Fails**
```
notify::Watcher error
  -> Log critical error
  -> Attempt to restart watcher
  -> If restart fails: Continue without hot-reload (degraded mode)
```

**4. Broadcaster Channel Full**
```
Slow client causes channel overflow
  -> Oldest messages dropped automatically
  -> Client will detect via polling fallback
  -> No server impact
```

### Client-Side Error Scenarios

**1. Stream Connection Lost**
```
Network interruption
  -> Stream errors out
  -> Automatic fallback to polling
  -> Log: "Config watch stream disconnected, using polling"
  -> Continue operation (degraded but functional)
```

**2. Polling Fails**
```
GetConfigVersion RPC error
  -> Log warning
  -> Retry on next interval (30s)
  -> UI continues with cached commands
```

**3. Command Refresh Fails**
```
ListCommands RPC error after update notification
  -> Show snackbar: "Failed to refresh commands"
  -> Retry after 5 seconds
  -> Keep showing old commands (stale but safe)
```

**4. Version Skew**
```
Client version 5 -> Server reloads to version 8 (missed versions 6, 7)
  -> Client detects version > cached_version
  -> Refresh triggered (final state is correct)
  -> Intermediate versions don't matter
```

---

## Testing Strategy

### Unit Tests

**Server (`src/config/watcher_test.rs`):**
```rust
#[tokio::test]
async fn test_config_reload_valid() {
    let tempdir = TempDir::new().unwrap();
    let config_path = tempdir.path().join("config.toml");

    // Write initial config
    std::fs::write(&config_path, "[[command]]\nid = \"test\"").unwrap();

    let watcher = ConfigWatcher::new(/* ... */);

    // Modify config
    std::fs::write(&config_path, "[[command]]\nid = \"test2\"").unwrap();

    // Wait for reload
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Assert config updated
    let config = watcher.config.read().unwrap();
    assert_eq!(config.command[0].id, "test2");
}

#[tokio::test]
async fn test_config_reload_invalid_keeps_old() {
    // Write invalid TOML
    std::fs::write(&config_path, "[[command]\ninvalid syntax").unwrap();

    tokio::time::sleep(Duration::from_secs(1)).await;

    // Assert old config still active
    let config = watcher.config.read().unwrap();
    assert_eq!(config.command[0].id, "original");
}

#[tokio::test]
async fn test_version_increments_on_reload() {
    let initial_version = version.load(Ordering::SeqCst);

    std::fs::write(&config_path, "[[command]]\nid = \"new\"").unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;

    let new_version = version.load(Ordering::SeqCst);
    assert_eq!(new_version, initial_version + 1);
}
```

**Client (`GrpcCommandRepositoryTest.kt`):**
```kotlin
@Test
fun `watchConfigUpdates emits on server notification`() = runTest {
    val repository = GrpcCommandRepository()
    val updates = mutableListOf<ConfigUpdate>()

    launch {
        repository.watchConfigUpdates().take(1).collect {
            updates.add(it)
        }
    }

    // Simulate server notification
    mockServer.sendConfigUpdate(version = 2UL)

    advanceUntilIdle()
    assertEquals(1, updates.size)
    assertEquals(2UL, updates[0].version)
}

@Test
fun `polling fallback activates on stream error`() = runTest {
    mockServer.failWatchStream()

    repository.startWatchingConfig("localhost", 50051)
    advanceTimeBy(31_000) // 30s + buffer

    // Assert GetConfigVersion called
    verify(mockServer).getConfigVersion(any())
}
```

### Integration Tests

**End-to-End Test:**
```rust
#[tokio::test]
async fn test_full_reload_flow() {
    // 1. Start server with initial config
    let server = start_test_server().await;

    // 2. Connect Android client
    let client = TestClient::connect(server.addr()).await;
    let mut stream = client.watch_config_updates().await.unwrap();

    // 3. Get initial command list
    let commands = client.list_commands().await.unwrap();
    assert_eq!(commands.len(), 1);
    let initial_version = commands.config_version;

    // 4. Modify config file
    modify_server_config(&server, add_command("new-cmd")).await;

    // 5. Wait for stream notification
    let notification = stream.next().await.unwrap().unwrap();
    assert_eq!(notification.config_version, initial_version + 1);

    // 6. Fetch updated commands
    let updated_commands = client.list_commands().await.unwrap();
    assert_eq!(updated_commands.len(), 2);
    assert!(updated_commands.iter().any(|c| c.id == "new-cmd"));
}
```

### Manual Testing Checklist

**Server:**
- [ ] Modify config.toml while server running, verify reload logged
- [ ] Add invalid syntax, verify error logged and old config kept
- [ ] Change server.port, verify warning logged and reload rejected
- [ ] Monitor server logs during rapid file edits (debouncing works)
- [ ] Kill file watcher process, verify degraded mode operation

**Client:**
- [ ] Connect to server, modify config, verify commands auto-refresh
- [ ] Disconnect network, verify polling fallback activates
- [ ] Add command on server, verify appears on Android within 30s
- [ ] Remove command on server, verify disappears from Android
- [ ] Open command list, modify config, verify UI updates without navigation

**Race Conditions:**
- [ ] Execute command immediately after deleting it from config
- [ ] Reload config during active command execution
- [ ] Multiple clients connected during config reload

---

## Implementation Phases

### Phase 1: Server Foundation (Week 1)

**Tasks:**
1. Add `notify` crate dependency to Cargo.toml
2. Change `Arc<Config>` to `Arc<RwLock<Config>>` throughout codebase
3. Implement `src/config/watcher.rs` with file watching
4. Implement `src/config/broadcaster.rs` with tokio::broadcast
5. Update `src/main.rs` to start watcher task
6. Add config version tracking (AtomicU64)
7. Write unit tests for watcher and broadcaster

**Deliverable:** Server can reload config on file change, logs version updates

**Testing:** Manual file editing, check logs for reload events

### Phase 2: Protocol and gRPC (Week 1-2)

**Tasks:**
1. Update `proto/handcontrol.proto` with new RPCs
2. Implement `GetConfigVersion` RPC in `src/grpc/config.rs`
3. Implement `WatchConfigUpdates` streaming RPC
4. Update `ListCommands` to include `config_version` field
5. Regenerate protobuf code for Rust and Android
6. Write integration tests for new RPCs

**Deliverable:** Server exposes config versioning and streaming APIs

**Testing:** Use grpcurl to call new RPCs, verify responses

### Phase 3: Android Client Integration (Week 2)

**Tasks:**
1. Update Android proto files (regenerate from proto/)
2. Implement `GrpcCommandRepository.watchConfigUpdates()`
3. Implement streaming subscription in repository
4. Implement polling fallback mechanism
5. Update `CommandListViewModel` to auto-refresh on updates
6. Write unit tests for repository and ViewModel
7. Manual UI testing

**Deliverable:** Android client auto-refreshes on config changes

**Testing:** Modify server config, verify Android updates within 30s

### Phase 4: Polish and Documentation (Week 2-3)

**Tasks:**
1. Add tracing/logging throughout
2. Improve error messages
3. Add telemetry for reload events
4. Update PRD.md with config reload section
5. Add example configs showing hot-reload behavior
6. Performance testing (reload latency, memory usage)
7. Security audit of implementation

**Deliverable:** Production-ready feature with documentation

**Testing:** Full manual test checklist, stress testing

---

## Performance Considerations

### Server

**Memory:**
- RwLock overhead: ~40 bytes per instance (negligible)
- Broadcast channel: 100 slots * ~24 bytes = 2.4 KB (negligible)
- Two copies of config during reload: ~10-50 KB temporary (acceptable)

**CPU:**
- File watcher: <1% CPU when idle, <5% during reload
- Config parsing: ~1-5ms for typical config
- Broadcast: O(n) where n = connected clients, ~0.1ms per client

**Latency:**
- File change to reload complete: ~500ms (debounce delay)
- Broadcast to all clients: <10ms
- Total client notification latency: <600ms

### Client

**Network:**
- Streaming: One persistent connection, ~100 bytes/update
- Polling: 1 request per 30s, ~50 bytes/request (minimal)

**Battery:**
- Streaming: Negligible (connection already open for other RPCs)
- Polling: ~0.1% battery per day (very low)

**Memory:**
- ConfigUpdate flow: ~1 KB buffer (negligible)

### Scalability

**Server capacity:**
- File watcher: 1 thread regardless of client count
- Broadcast: O(1) memory per update, O(n) CPU per broadcast
- **Bottleneck:** 1000+ simultaneous streaming clients
- **Mitigation:** Polling fallback for low-priority clients

---

## Alternatives Considered

### Alternative 1: Push Notifications (FCM)

**Approach:** Use Firebase Cloud Messaging to push config updates.

**Pros:**
- Works when app in background
- Lower battery usage

**Cons:**
- Requires Google Play Services (not all devices)
- Adds external dependency
- Higher latency (5-30 seconds)
- Privacy concerns (Google sees update events)

**Decision:** Rejected due to added complexity and dependency.

### Alternative 2: Client-Only Polling

**Approach:** No streaming, clients poll every 10-30 seconds.

**Pros:**
- Simpler implementation
- No connection management

**Cons:**
- Higher latency (10-30s average)
- More network requests
- Higher battery usage

**Decision:** Rejected, but kept as fallback mechanism.

### Alternative 3: WebSocket

**Approach:** Use WebSocket for bidirectional communication.

**Pros:**
- Standard protocol
- Good browser support

**Cons:**
- Extra dependency (separate from gRPC)
- Duplicates gRPC transport
- More complex than gRPC streaming

**Decision:** Rejected, gRPC streaming is sufficient.

---

## Open Questions

1. **Debounce duration:** Is 500ms optimal? Should it be configurable?
   - **Recommendation:** Start with 500ms, make configurable if users request it

2. **Broadcast channel capacity:** Is 100 slots sufficient?
   - **Recommendation:** 100 is generous, monitor in production

3. **Polling interval:** Is 30s appropriate?
   - **Recommendation:** 30s balances latency vs battery, consider making configurable

4. **Version counter overflow:** What happens at 2^64 updates?
   - **Answer:** 2^64 updates at 1/sec = 584 billion years, not a concern

5. **Partial config reload:** Should we support reloading individual command files?
   - **Recommendation:** Not in v1, add if users request it

---

## Success Metrics

**Quantitative:**
- Config reload latency: <1 second (file change to server reload)
- Client notification latency: <30 seconds average
- Error rate: <0.1% (invalid configs rolled back)
- Server uptime: Unaffected (no restarts required)

**Qualitative:**
- User feedback: "Commands appear instantly after editing config"
- Developer experience: "Hot-reload makes iteration much faster"
- Reliability: "Never had to restart server for config changes"

---

## Migration Path

### For Existing Users

**No breaking changes:**
- Existing configs work unchanged
- Old clients continue to work (just won't auto-refresh)
- Server starts without issues

**Upgrade path:**
1. Update server binary (auto-detects config changes)
2. Update Android app (gets auto-refresh feature)
3. No config file changes needed

### Backwards Compatibility

**Old server, new client:**
- Client calls `WatchConfigUpdates`, server returns UNIMPLEMENTED
- Client falls back to manual refresh (existing behavior)

**New server, old client:**
- Server reloads config, but client never fetches updates
- Client must manually refresh (existing behavior)

**Recommendation:** Encourage users to update both server and client for best experience.

---

## Conclusion

This feature significantly improves the HandControl developer and user experience by eliminating server restarts for config changes. The implementation follows the project's conventions (async Rust, Kotlin coroutines, MVVM), maintains security guarantees (mTLS, validation), and provides graceful degradation (polling fallback).

The hybrid streaming + polling approach balances low latency with reliability, and the commands-only scope keeps the implementation simple while addressing the most common use case.

**Next Steps:**
1. Review this specification with stakeholders
2. Approve implementation plan
3. Begin Phase 1 (server foundation)
4. Iterate based on testing feedback
