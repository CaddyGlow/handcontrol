# Feature Plan: QR Enrollment via Relay

**Version:** 0.1  
**Date:** 2025-03-21  
**Status:** Draft  
**Owner:** HandControl Core  
**Reviewers:** Android, CLI/TUI, Relay, Security

---

## 1. Overview

### Problem Statement
QR enrollment currently assumes the enrolling client can reach the server over one of the advertised LAN IPs. When the client is on a different network (mobile data, guest Wi-Fi, remote office), enrollment fails even though the platform already ships an optional relay (`relay/`, `server/src/relay/`) that can broker tunnels. We need an end-to-end plan to let QR enrollment provision the relay path automatically, so newly enrolled devices can connect immediately regardless of network topology.

### Goals
- Encode relay connectivity metadata in the QR payload when the server is relay-enabled.
- Provision short-lived relay credentials for the enrolling client and persist them alongside the mutual TLS material.
- Teach all HandControl clients (Rust CLI/TUI, Android) to parse the relay block and fall back to the relay whenever direct dialing fails.
- Ensure the enrollment UX communicates when the relay is required vs optional.
- Keep security guarantees: relay never terminates TLS, credentials scoped and revocable, no downgrade path.

### Non-Goals
- Building public discovery for relay hosts (operator supplies relay URL out-of-band).
- Designing multi-relay failover; a single relay endpoint per server is sufficient for this iteration.
- Modifying approval-mode enrollment beyond mirroring the same relay payload delivery.
- Shipping iOS or desktop GUI clients (future work can reuse this plan).

### Scope
- Rust server, CLI, TUI, client-lib crates.
- Android app (Jetpack Compose client).
- Relay service configuration and token issuance.
- Documentation and operator workflows.

---

## 2. Current State Analysis
- Server already includes a relay block in QR payloads when `relay.include_in_enrollment = true` (`server/src/utils/qr.rs`, `server/src/grpc/server.rs:84-117,307-346`).
- Relay token issuance and WebSocket tunnelling are implemented and tested (`relay/PROGRESS.md`, `server/src/relay`).
- `client-lib` can persist relay information and use it in the connection strategy, but QR parsing currently treats the relay block as optional metadata; CLI/TUI do not yet attempt relay fallback for enrollment traffic.
- Android integration is nearing completion (per `relay/PROGRESS.md`), but still lacks automated tests and final UX polish around mixed direct/relay modes.
- Documentation mentions future relay fields in QR payloads (`docs/MULTI_IP_ENROLLMENT.md`) but does not describe the end-to-end enrollment workflow or operational guidance.

---

## 3. Functional Requirements

1. **QR Payload Parity**
   - Server MUST include relay metadata (`relay_url`, `relay_token`, `relay_required`, TLS hints) whenever relay enrollment is enabled.
   - Payload MUST remain backward compatible: legacy clients ignore the `relay` object.

2. **Credential Lifecycle**
   - Relay tokens issued during enrollment MUST be scoped to the enrolling client ID and expire within configurable TTL (default 24h).
   - Server MUST rotate relay tokens when a client renews certificates or is revoked.

3. **Client Consumption**
   - CLI/TUI/Android MUST persist relay credentials with the enrolled server record.
   - Connection managers MUST attempt direct addresses first (unless `relay_required` or user preference flips) and fall back to relay automatically.
   - Enrollment flows MUST surface relay availability (badge/icon/text).

4. **Security & Trust**
   - Clients MUST validate server certificate fingerprints and server IDs before accepting relay credentials.
   - Relay TLS handling MUST respect `allow_self_signed_tls` and optional pinned fingerprints from payload/config.

5. **Operations**
   - Config MUST expose knobs for relay token TTL, include-in-QR toggle, and TLS expectations.
   - Server MUST log relay token issuance and consumption for audit.

---

## 4. Technical Design

### 4.1 QR Payload Schema
- Continue using `EnrollmentQrPayload` (`server/src/utils/qr.rs`) with the existing `relay` struct:
  ```json
  {
    "ips": ["192.168.1.42", "2001:db8::5"],
    "port": 50051,
    "cert_fingerprint": "SHA256:...",
    "enrollment_token": "...",
    "server_id": "...",
    "valid_until": "2025-03-21T22:15:30Z",
    "relay": {
      "relay_url": "wss://relay.example.com",
      "relay_token": "jwt",
      "relay_required": false,
      "allow_self_signed_tls": false,
      "pinned_cert_sha256": null
    }
  }
  ```
- Define JSON contract formally in docs and sample responses (README + `docs/PROJECT_STRUCTURE.md`).
- Update CLI `handcontrol server enroll --qr` output to visibly group relay information (colored block or clear section).

### 4.2 Server Enhancements
1. **Relay Eligibility Gate**
   - Extend config validation to ensure `relay_server_url` and signing keys exist whenever `include_in_enrollment` is true; fail fast otherwise.
2. **Token Scope**
   - Amend `generate_relay_token` to embed binding metadata (`binding_type`, `binding_value`) that records whether the JWT is tied to an enrollment token or a persistent client ID, preventing reuse across contexts.
   - Include `server_audience` alongside the existing `server_id` claim so the relay can assert which server issued the token while still validating the `aud` (relay URL).
   - Emit structured logs (`info!`) with server ID, binding info, and expires-at (no secrets).
3. **Approval Mode Parity**
   - Confirm `CheckPairingStatusResponse` delivers relay info on approval completion; document in `docs/FEATURE_PERSISTENT_ENROLLMENT.md`.
4. **Revocation Hooks**
   - On client revoke (`handcontrol server clients revoke`), delete associated relay tokens and optionally notify relay (WebSocket control channel message).
5. **Configuration Docs**
   - Add relay enrollment guidance to `docs/FEATURE_RELAY.md` and server admin guide (sample config, TLS pinning instructions).

### 4.3 Relay Service
1. **Audience & TLS Guidance**
   - Document expectation that relay is reachable via `relay_url` and presents TLS cert matching payload hints.
2. **Token Validation**
   - Ensure relay validates audience/issuer that now may include enrollment token claim.
3. **Monitoring**
   - Expose metrics/logs for enrollment-sourced tunnels (e.g., handshake counts, failures) to aid operators.
4. **Key Rotation**
   - Provide script/instructions for rotating relay auth secret without breaking existing clients (dual-key window).

### 4.4 Client Implementations

#### Rust client-lib (CLI/TUI)
- **Enrollment Parsing**: `client-lib/src/enrollment.rs` already lifts relay info; ensure it is persisted in `ServerRegistryEntry` and returned to callers.
- **Connection Strategy**: `client-lib/src/grpc_client.rs` must honor `RelayBehaviorConfig` defaults while ensuring at least one relay attempt when credentials exist.
- **Config UX**: Extend CLI/TUI config commands to toggle `prefer_relay`, `relay_only_mode`, and explain behavior.
- **Status Surfacing**: CLI `server list` should mark relay-capable servers; TUI should display connection mode indicator (direct vs relay).
- **Token Refresh**: When CLI/TUI trigger certificate renewal, request new relay token (server returns via response); update registry.

#### Android
- **QR Parsing**: Ensure `GrpcEnrollmentRepository` escrow relay info and writes to Room (`relayEnabled`, `relayUrl`, `relayToken`, TLS flags).
- **Connection Mode UI**: Finalize UI badges showing fallback status and last connection mode.
- **Preferences**: Offer global connection mode preference (auto/direct-only/relay-only) to mirror CLI/TUI behaviour.
- **Background Renewals**: WorkManager job should refresh relay token alongside certificate renewal.
- **Error Handling**: Present human-readable errors when relay tunnel fails (expired token, TLS mismatch) and prompt to re-enroll if necessary.
- **Testing**: Add instrumentation tests using mock relay + local server; cover direct failover, relay-only scenarios, token expiry.

#### Future Clients (Out of Scope)
- Document expectations so new clients can plug into same enrollment payload without redesign.

### 4.5 Shared Libraries
- Update protobuf comment on `RelayInfo` (currently says "future use") to reflect active usage.
- Introduce serialization tests ensuring QR payload remains backward compatible (serde round-trips with/without relay block).
- Add integration test harness under `examples/` or `tests/` to simulate enrollment -> relay connection using only Rust components.

### 4.6 Enrollment Flow (Happy Path & Fallback)
1. **Server Preparation**
   - Operator enables relay in config (`relay.include_in_enrollment = true`), provides `relay_server_url`, signing key, and TLS hints.
   - Server boots, validates relay readiness, and registers signing key with relay service.
2. **QR Generation**
   - Operator requests enrollment QR (CLI command or TUI action).
   - Server mints enrollment token, derives provisional client ID, signs relay token scoped to that ID, and returns payload with `relay` block.
   - QR encoder embeds relay fields; printed/onscreen QR contains both LAN addresses and relay metadata.
3. **Client Scan**
   - Client decodes QR, validates payload shape/version, persists provisional enrollment state (including relay credentials encrypted at rest).
   - Client initiates enrollment RPC over direct connection if `relay_required = false`; otherwise it skips to relay dial.
4. **Connection Attempt Ordering**
   - Client attempts direct gRPC using IPv4/IPv6 addresses in order of operator preference.
   - Upon exhaustion or explicit `relay_required = true`, client upgrades to relay: open WebSocket, exchange relay token, initiate gRPC tunnel.
5. **Enrollment Completion**
   - Enrollment RPC succeeds, server issues long-lived client certificate and refreshed relay token if needed.
   - Client stores final credentials + relay metadata, marks server as relay-capable, and surfaces UX messaging (badge/tooltips).
6. **Post-Enrollment Usage**
   - Subsequent RPCs follow the same connection ordering; relay metadata is refreshed on certificate renewal or manual refresh actions.

### 4.7 Failure Handling & Edge Cases
- **Relay Token Expiry During Enrollment**: Client should catch `Unauthenticated` from relay, request fresh QR or polling fallback; server logs token expiry with subject ID.
- **Relay Misconfiguration**: If relay handshake fails due to TLS mismatch, client surfaces actionable error (showing expected vs provided fingerprint) and suggests verifying server config.
- **Direct Path Available After Relay**: If client initially connects via relay but later detects working direct IP, it should prefer direct on next attempt while keeping relay as safety net.
- **Multi-Server Enrollment**: Clients must isolate relay tokens by server ID to prevent cross-server reuse.
- **Offline QR Usage**: Payload includes `valid_until` (existing field); clients must refuse enrollment once TTL elapses to avoid stale relay tokens.
- **Approval Mode Delays**: When approval flow takes longer than relay token TTL, server must re-issue fresh relay token alongside approval response.

### 4.8 Migration Strategy
- **Server Rollout**
  - Ship relay enhancements behind config flag; default remains disabled to avoid surprising operators.
  - Provide migration script that audits current configs and suggests relay parameters.
- **Client Backward Compatibility**
  - Legacy clients ignore `relay` block; no protocol changes required.
  - New clients must handle absence of relay data gracefully (direct-only flow).
- **Relay Service Updates**
  - Deploy relay validating new token claims before flipping server flag; maintain dual validation logic during rollout window.
  - Monitor relay logs for new claim fields; rollback plan includes disabling enrollment flag and revoking issued tokens.

---

## 5. Security Considerations
- **End-to-End TLS**: Relay continues to forward opaque TLS frames (`relay_e2e_encryption_plan.md`); tests must assert no plaintext logging.
- **Token TTL & Revocation**: Short TTL plus mandatory renewal on cert refresh reduces exposure. Consider storing issued token IDs to support early revocation.
- **Self-Signed Relay**: When `allow_self_signed_tls` is true, require `pinned_cert_sha256` to prevent MITM; clients must refuse enrollment if fingerprint missing.
- **Downgrade Protection**: Clients should remember if relay was required; if later QR omits relay unexpectedly, warn operator (possible misconfiguration).
- **Audit Trail**: Log enrollment events with relay usage flag for incident response.

---

## 6. User Experience & Documentation
- Update CLI help (`handcontrol enroll qr --help`) describing relay fallback.
- Refresh CLI server listings to highlight relay availability/requirements and surface relay URLs for quick inspection.
- Show relay badges in the TUI server pane so operators can see availability without drilling into details.
- Add README section "Enrolling Across Networks" with step-by-step operator + client instructions.
- Provide Android onboarding copy explaining relay usage and privacy (relay cannot read commands).
- Extend `docs/SECURITY.md` with relay enrollment threat model subsection.

---

## 7. Testing Strategy
- **Unit Tests**
  - QR payload serialization/deserialization with relay block (Rust and Android).
  - Relay token scope validation (server-side).
  - Connection strategy fallbacks in client-lib.
- **Integration Tests**
  - Rust end-to-end: spin up server + relay (mock) + CLI enrollment, assert relay connection works when server IP unreachable.
  - Android instrumentation: use emulator -> mock relay to verify fallback.
- **Manual QA**
  - Direct network (same Wi-Fi) to ensure relay optional.
  - Mobile hotspot where direct fails to ensure automatic relay fallback.
  - Expired token scenario requiring re-enrollment.

---

## 8. Rollout Plan
1. **Phase 1 - Server & Relay Hardening**
   - Ship server changes (token scope, logging, docs) and relay validation updates.
   - Release new protobuf + client-lib crate.
2. **Phase 2 - Rust Clients**
   - Update CLI/TUI to consume relay info, add UX feedback.
   - Release workspace version bump.
3. **Phase 3 - Android**
   - Land UI polish, background renewals, tests.
   - Publish beta build for field testing.
4. **Phase 4 - Documentation & Operator Guides**
   - Merge docs, sample configs, troubleshooting flowcharts.
   - Provide changelog entry emphasising new QR relay flow.
5. **Phase 5 - Monitoring & Feedback**
- Observe telemetry (relay usage counts, failure rates); collect user feedback for further tuning (multi-relay, priority toggles).

---

## 9. Open Questions
- Should relay tokens be long-lived until explicit revocation, or renewed alongside client certificates? **Decision:** Keep tokens short-lived and renew alongside certificate lifecycle.
- Do we need per-client bandwidth quotas on the relay to prevent abuse once QR leaks? **Decision:** No quotas for this release; rely on existing auth and monitoring.
- How to surface relay connectivity health in CLI/TUI dashboards (e.g., latency stats)? **Decision:** Track and display latency plus error rate; leave throughput for future work.
- Should server proactively test relay reachability before embedding info in QR? **Decision:** No proactive probe; operators accept responsibility for relay uptime.

---

## 10. Success Criteria
- New enrollment succeeds from a mobile network without manual configuration changes.
- Clients automatically fall back to relay on subsequent connections when LAN IPs fail.
- Operators can audit which clients use relay connectivity.
- No regressions for pure LAN deployments; QR payload remains backwards compatible.

---

## 11. Dependencies & Risks
- **Relay Infrastructure Availability**: Any outage in relay service blocks cross-network enrollment; ensure HA deployment before launch.
- **Token Signing Key Management**: Compromise of relay signing key enables malicious tunnel creation; store in HSM-compatible keystore and rotate on schedule.
- **Client Update Cadence**: All clients must ship relay-aware builds before operators turn on relay-only enrollment; coordinate release timelines.
- **Config Drift**: Divergence between documented config and deployed values can silently break relay fallback; add config lints and operator runbooks.
- **Analytics Privacy**: Telemetry on relay usage must avoid leaking sensitive user commands; restrict metrics to counts/durations.
