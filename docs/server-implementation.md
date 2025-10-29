# HandControl Server Implementation Guide

## Overview

- Purpose: translate the product requirements (docs/PRD.md) into actionable Rust server workstreams.
- Scope: Rust/Tokio server, gRPC/mTLS stack, enrollment flows (QR + Approval), command execution, mDNS discovery.
- Status: Draft; update alongside feature delivery.

## Architecture & Modules

### Layering

- `config`: TOML parsing, validation, default configuration generation.
- `security`: certificate management, enrollment logic (QR + Approval), verification codes, mTLS setup.
- `grpc`: Tonic server, RPC implementations, authentication middleware.
- `commands`: shell command execution, parameter substitution, output streaming.
- `mdns`: service discovery and advertisement.
- `notifications`: OS-specific notification systems for approval mode.
- `storage`: authorized clients registry, certificate storage, platform paths.
- `utils`: logging setup, QR code generation, shared utilities.

### Technology Selections

- Runtime: Tokio (async).
- gRPC: Tonic + Prost.
- TLS: rustls (pure Rust, no OpenSSL dependency).
- Certificates: rcgen for self-signed generation.
- Config: TOML via `toml` crate.
- Paths: `directories` crate (XDG on Linux, platform-specific elsewhere).
- mDNS: `mdns-sd`.
- Logging: `tracing` + `tracing-subscriber` (structured logging).
- Errors: `anyhow` (application), `thiserror` (domain-specific).

## Tooling & Build Setup

1. Protobuf tooling: `build.rs` with `tonic-build` + `prost-build`.
2. Dependencies already added to `Cargo.toml` (see PRD table for versions).
3. Platform-specific notification crates:
   - Linux: `notify-rust` (D-Bus).
   - Windows: `windows` crate (Toast notifications).
   - macOS: `mac-notification-sys` (Notification Center).
4. CI tasks: `cargo test`, `cargo clippy`, `cargo fmt --check`.
5. Nix development shell defined in `flake.nix` provides toolchain + dependencies.

## Security & Certificates

- Generate server certificate on first run using `rcgen` (self-signed, ECDSA P-256, 10-year validity).
- Store in platform-specific config directory (`~/.config/handcontrol/` on Linux).
- Client certificates received during enrollment stored in `authorized_clients/` directory.
- **Algorithm:** ECDSA P-256 (secp256r1) for both server and client certificates
  - Smaller certificate sizes (better for network transmission)
  - Better performance on mobile devices
  - Hardware-backed support on Android Keystore
  - Equivalent security to RSA-3072
- Certificate fingerprints (SHA256) used for verification and mDNS advertisement.
- mTLS enforced for all RPCs except enrollment endpoints.
- Enrollment tokens (QR mode) are single-use UUIDs with 5-minute TTL.
- Verification codes (Approval mode) prevent MITM attacks via out-of-band verification.

## Enrollment Flows

### QR Code Mode

1. Generate one-time enrollment token (UUID) with expiry.
2. Create QR code payload (JSON) containing: IP, port, cert fingerprint, token, server ID.
3. Display QR code in terminal (ASCII art via qr2term or similar).
4. Accept `Enroll` RPC with token validation.
5. Verify token not expired/used, store client certificate, invalidate token.
6. Return success with assigned client ID.

### Approval Mode

1. Accept `RequestPairing` RPC with client certificate and verification code.
2. Server computes its own verification code from certificates.
3. MANDATORY: Verify client's code matches server's before proceeding.
4. If mismatch: return error immediately (possible MITM).
5. If match: show OS notification with device name and verification code.
6. User approves/rejects via notification action buttons.
7. Android polls `CheckPairingStatus` until approved/rejected/timeout.
8. On approval: store client certificate, return client ID.
9. On rejection/timeout: clean up pending request.

## Command Lifecycle

1. Parse TOML config file on startup.
2. Validate command definitions (required fields, parameter types).
3. Serve command list via `ListCommands` RPC (transform to protobuf).
4. Accept `ExecuteCommand` RPC with command ID and parameters.
5. Validate parameters (types, ranges, regex validation).
6. Substitute parameters into shell command with proper escaping.
7. Spawn process, stream stdout/stderr chunks via gRPC streaming.
8. Return final exit code.
9. Enforce timeout (configurable per command).
10. Log all executions with client ID, command ID, exit code.

## Network Discovery (mDNS)

1. Register service: `_handcontrol._tcp.local.`
2. Instance name: system hostname (or configured name).
3. TXT records:
   - `version=1.0`
   - `server_id=<uuid>`
   - `cert_fingerprint=<SHA256>`
4. Port from config (default 50051).
5. Auto-restart mDNS service on config reload.

## Configuration Management

### File Location

- Linux: `~/.config/handcontrol/config.toml`
- Windows: `%APPDATA%\handcontrol\config.toml`
- macOS: `~/Library/Application Support/handcontrol/config.toml`

### Config Loading

1. Check if config exists; if not, generate default with example commands.
2. Parse TOML, validate schema.
3. Expand `~` in paths to home directory.
4. Validate command definitions (no duplicate IDs, valid parameter types).
5. Support hot reload (watch file changes, reload config without restart - future enhancement).

### Default Commands

Generate platform-specific example commands:
- Linux: lock-screen (loginctl), suspend (systemctl), volume (pactl).
- Windows: lock (rundll32), shutdown, volume (nircmd).
- macOS: lock (CGSession), volume (osascript).

## Logging Strategy

- Use `tracing` macros (error!, warn!, info!, debug!, trace!).
- Initialize `tracing-subscriber` with env filter (RUST_LOG).
- Log format: timestamp, level, target, message.
- NEVER log sensitive data: tokens, verification codes, private keys.
- Log important events:
  - Server startup (port, config location).
  - Enrollment attempts (success/failure, device name).
  - Pairing requests (device name, approval outcome).
  - Command executions (command ID, client ID, exit code).
  - Errors (authentication failures, command timeouts).

## Error Handling

- Application errors: use `anyhow::Result` for main/top-level functions.
- Domain errors: define `thiserror` enums per module.
- gRPC status codes:
  - `UNAUTHENTICATED`: invalid/missing client certificate.
  - `PERMISSION_DENIED`: enrollment token invalid/expired.
  - `NOT_FOUND`: command ID doesn't exist.
  - `INVALID_ARGUMENT`: invalid parameters.
  - `DEADLINE_EXCEEDED`: command timeout.
  - `INTERNAL`: server error.

## Testing Strategy

- Unit tests: config parsing, parameter substitution, verification code generation, token expiry.
- Integration tests: enrollment flows, command execution, mTLS handshake.
- Security tests: certificate validation, token replay, MITM detection, shell injection attempts.
- Platform tests: run on Linux/Windows/macOS to verify OS-specific code.

## Implementation Phases

### Phase 1: Foundation (Essential Infrastructure)

**Goal:** Basic server that can start, load config, and handle core infrastructure.

1. Project structure setup:
   - Create module directories (config/, security/, grpc/, etc.).
   - Add `mod.rs` files for each module.
   - Update `main.rs` with basic initialization.

2. Configuration module:
   - TOML schema structs with serde.
   - Parse config file.
   - Platform-specific path resolution (directories crate).
   - Generate default config if not exists.
   - Validation logic.

3. Logging setup:
   - Initialize tracing-subscriber.
   - Configure format and filtering.

4. Error types:
   - Define error enums with thiserror.

**Deliverables:**
- Server starts, loads config, logs to console.
- Default config generation works.
- Platform paths resolved correctly.

**Tests:**
- Config parsing with valid/invalid TOML.
- Path resolution on different platforms.
- Default config generation.

---

### Phase 2: Security & Certificates

**Goal:** Certificate generation, storage, and mTLS setup.

1. Certificate generation:
   - Generate self-signed server certificate using `rcgen` with ECDSA P-256 algorithm.
   - Key pair: ECDSA with secp256r1 curve (equivalent security to RSA-3072).
   - Store certificate and key in config directory (PEM format).
   - Load existing certificates on startup.
   - Compute SHA256 fingerprint for mDNS and verification.

2. Client certificate storage:
   - Create authorized_clients/ directory.
   - Store client certificates received from Android (DER format, ECDSA P-256).
   - Maintain metadata.toml registry with client ID, name, fingerprint.
   - Add/remove/list clients operations.

3. mTLS configuration:
   - Configure rustls with server ECDSA certificate.
   - Configure client certificate validation (verify ECDSA signatures).
   - Custom certificate verifier: check against authorized_clients/ list.
   - Support ECDSA cipher suites in rustls config.

4. Enrollment token management:
   - Generate UUIDs with expiry timestamps.
   - Token validation (not expired, single-use).
   - Token cleanup (remove expired).

5. Verification code generation:
   - Implement algorithm from PRD.
   - SHA256(client_fp || server_fp || server_id) -> first 6 digits.
   - Format as XXX-XXX.

**Deliverables:**
- Server generates and loads ECDSA P-256 certificates.
- mTLS enforced on gRPC server with ECDSA cipher suites.
- Enrollment tokens work correctly.
- Verification codes generated and validated.

**Tests:**
- ECDSA certificate generation and loading.
- Certificate fingerprint computation (SHA256).
- Token expiry logic.
- Verification code algorithm (matches Android implementation).
- mTLS handshake with ECDSA client certificates.

---

### Phase 3: gRPC Server & Protocol

**Goal:** gRPC server running with protocol buffers.

1. Protocol buffer definition:
   - Create proto/handcontrol.proto.
   - Define service and messages per PRD.

2. Build script:
   - Create build.rs with tonic-build.
   - Generate Rust code from proto.

3. gRPC server setup:
   - Tokio runtime initialization.
   - Tonic server with mTLS.
   - Bind to configured address/port.

4. Health check endpoint:
   - Implement GetServerInfo RPC.
   - Return server ID, hostname, version, OS.

**Deliverables:**
- gRPC server running on configured port.
- GetServerInfo RPC works.
- mTLS connection successful.

**Tests:**
- Server starts and binds.
- GetServerInfo returns correct data.
- mTLS connection from test client.

---

### Phase 4: Enrollment - QR Code Mode

**Goal:** QR code based enrollment working end-to-end.

1. Enrollment RPC implementation:
   - Implement Enroll RPC handler.
   - Token validation.
   - Store client certificate.
   - Return client ID.

2. QR code generation:
   - Generate enrollment session (token + metadata).
   - Create JSON payload.
   - Generate QR code (ASCII art for terminal).
   - Display to user.

3. CLI command for enrollment:
   - Add subcommand: `handcontrol enroll --qr`.
   - Generate and display QR code.
   - Wait for enrollment or timeout.

**Deliverables:**
- QR code displayed in terminal.
- Android client can scan and enroll.
- Client certificate stored successfully.

**Tests:**
- Token validation (valid/expired/used).
- Client certificate storage.
- JSON payload parsing.

---

### Phase 5: Enrollment - Approval Mode

**Goal:** Interactive approval-based enrollment.

1. Pairing request RPC:
   - Implement RequestPairing RPC.
   - Verification code validation.
   - Create pending pairing request.
   - Return pairing request ID.

2. Notification system:
   - Platform detection.
   - Linux: D-Bus notifications (notify-rust).
   - Windows: Toast notifications (windows crate).
   - macOS: Notification Center (mac-notification-sys).
   - Fallback: terminal prompt.

3. Pairing status RPC:
   - Implement CheckPairingStatus RPC.
   - Poll pending request status.
   - Return approved/rejected/timeout.

4. User approval flow:
   - Show notification with verification code.
   - Action buttons: Accept/Reject.
   - Handle user response.
   - Store client certificate on approval.

**Deliverables:**
- RequestPairing RPC works.
- OS notifications displayed.
- User can approve/reject.
- Client certificate stored on approval.

**Tests:**
- Verification code validation.
- Pairing timeout handling.
- User approval/rejection.
- Notification fallback.

---

### Phase 6: Command Execution

**Goal:** Execute commands and stream output.

1. Command parsing:
   - Parse command definitions from config.
   - Validate parameter schemas.

2. ListCommands RPC:
   - Transform config to protobuf.
   - Return command list.

3. ExecuteCommand RPC:
   - Parameter validation.
   - Parameter substitution with escaping.
   - Spawn shell process.
   - Stream stdout/stderr chunks.
   - Return exit code.
   - Enforce timeout.

4. Shell command executor:
   - Cross-platform shell detection.
   - Linux/macOS: /bin/sh
   - Windows: cmd.exe or powershell.
   - Process management.
   - Output capture and streaming.

**Deliverables:**
- ListCommands returns configured commands.
- ExecuteCommand spawns process and streams output.
- Command timeout enforced.
- Exit code returned.

**Tests:**
- Parameter validation (types, ranges, regex).
- Parameter substitution.
- Shell injection prevention.
- Timeout enforcement.
- Output streaming.

---

### Phase 7: Network Discovery (mDNS)

**Goal:** Server discoverable via mDNS.

1. mDNS service registration:
   - Register `_handcontrol._tcp.local.`.
   - Set instance name (hostname or config).
   - Set port from config.
   - Add TXT records (version, server_id, cert_fingerprint).

2. Service lifecycle:
   - Start on server startup.
   - Shutdown on server stop.
   - Update on config reload.

**Deliverables:**
- Server advertises on local network.
- Android client discovers server.
- TXT records correct.

**Tests:**
- Service registration.
- TXT record content.
- Service shutdown.

---

### Phase 8: Platform-Specific Features

**Goal:** Platform-specific optimizations and integrations.

1. Linux:
   - Systemd service file.
   - D-Bus notification support.

2. Windows:
   - Windows Service support (optional).
   - Toast notifications.

3. macOS:
   - launchd plist.
   - Notification Center support.

**Deliverables:**
- Installation guides per platform.
- Service files/plists provided.
- Platform-specific features working.

---

### Phase 9: Polish & Production Readiness

**Goal:** Production-ready server with all features complete.

1. Documentation:
   - Update PRD with implementation notes.
   - User guide (installation, configuration, usage).
   - Troubleshooting guide.

2. Example configurations:
   - Basic config (minimal commands).
   - Advanced config (all features).
   - Platform-specific examples.

3. CLI enhancements:
   - List enrolled devices.
   - Revoke device.
   - Regenerate server certificate.
   - Config validation command.

4. Performance:
   - Profile memory usage.
   - Optimize command execution.
   - Connection pooling.

5. Security audit:
   - Review all authentication checks.
   - Verify no sensitive data logged.
   - Test MITM scenarios.
   - Shell injection tests.

**Deliverables:**
- Complete documentation.
- Production-ready server.
- All tests passing.
- Security audit complete.

---

## Current Status

**Completed:**
- Dependencies added to Cargo.toml.
- Project structure defined.
- Documentation (PRD, Security, Project Structure) updated with ECDSA P-256 details.
- **Phase 1: Foundation - COMPLETE**
  - Module structure created (config/, security/, grpc/, commands/, mdns/, notifications/, storage/, utils/)
  - Configuration module: TOML parsing, validation, default generation
  - Logging infrastructure: tracing + tracing-subscriber with env filter
  - Platform-specific paths: XDG directories on Linux
  - Main initialization: config loading, validation, startup logging
  - All 11 unit tests passing
  - Server starts successfully and generates default config

- **Phase 2: Security & Certificates - COMPLETE**
  - ECDSA P-256 certificate generation using `rcgen`
  - Certificate storage and loading (PEM format)
  - SHA256 fingerprint computation
  - Client certificate storage in authorized_clients/ directory
  - Client metadata registry with TOML persistence
  - Enrollment token management (generation, validation, expiry, single-use)
  - Verification code generation (SHA256-based, 6-digit format XXX-XXX)
  - Main.rs integration: automatic certificate generation on first run
  - All 39 unit tests passing
  - Server generates ECDSA P-256 certificate and displays fingerprint

**In Progress:**
- Phase 3: gRPC Server & Protocol (next step).

**Pending:**
- mTLS configuration (requires protobuf definitions from Phase 3)
- Phases 4-9.

---

## Open Questions

1. Should server support hot config reload, or require restart?
2. Maximum command output size to prevent memory exhaustion?
3. Should server persist command execution history?
4. CLI: interactive shell mode vs single commands?
5. Logging: file logging in addition to stdout?

---

## Documentation Deliverables

- Update PRD with implementation notes as features land.
- Maintain user-facing installation guide (per platform).
- Create administrator guide (device management, security best practices).
- Document troubleshooting (enrollment failures, certificate issues, network discovery).

---

## Dependencies Verification

Current Cargo.toml matches PRD requirements. All crates use pure-Rust implementations for cross-compilation compatibility.

Missing platform-specific crates (to be added later):
- Linux: `notify-rust` (Phase 5).
- Windows: `windows` crate (Phase 5).
- macOS: `mac-notification-sys` (Phase 5).

---

## Success Criteria

Phase completion criteria:

1. Foundation: Server starts, loads config, logs correctly.
2. Security: mTLS working, certificates generated/stored.
3. gRPC: Server responds to RPCs over mTLS.
4. QR Enrollment: Complete enrollment flow works.
5. Approval Enrollment: Interactive approval with notifications.
6. Commands: Execute commands, stream output, return exit code.
7. mDNS: Server discoverable on network.
8. Platform: Service files and platform features complete.
9. Polish: Documentation, examples, CLI tools ready.

Project is production-ready when all phases complete and security audit passes.
