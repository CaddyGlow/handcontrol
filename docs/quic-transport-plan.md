# QUIC + WebSocket Transport Integration Plan

## 1. Objectives
- Offer QUIC as an optional transport next to the existing WebSocket relay without breaking current clients.
- Improve resilience to lossy/high-latency networks and enable mobility features (IP roam, quick resume).
- Maintain a single application protocol and auth model regardless of transport.

## 2. Background
- Current relay relies on WebSockets (TCP) for bi-directional messaging between clients and the server.
- WebSocket path already handles session auth, heartbeats, telemetry, and command streams.
- QUIC provides multiplexed, TLS 1.3 encrypted streams over UDP with built-in congestion control and connection migration.
- Cloudflare Tunnel cannot pass raw QUIC; only direct internet exposure or Cloudflare Spectrum (enterprise) can carry QUIC traffic.

## 3. Requirements & Success Criteria
- **Feature parity:** Every relay capability available over WebSocket must remain available over QUIC.
- **Backward compatibility:** Existing clients continue to function without modifications.
- **Negotiation:** Client chooses transport (config flag, env, CLI option) with server-side allow/deny lists.
- **Security:** TLS 1.3 via QUIC stack, certificate rotation, optional client auth, and token binding identical to WebSocket flow.
- **Observability:** Metrics and logs to compare transport performance, track handshakes, loss, reconnects.
- **Operational safety:** Rollout guarded by feature flags; easy fallback to WebSocket-only.

## 4. Architecture Overview
### 4.1 Transport Abstraction
- Introduce a transport-neutral interface (e.g., `TransportSession` trait) covering connect, authenticate, send, receive, heartbeat, close.
- Move application framing (protobuf or JSON), auth tokens, and session lifecycle above the transport boundary.
- Provide implementations for WebSocket and QUIC transports that share the same protocol handlers.
- Create `server/src/relay/transport.rs` with the shared trait and adapters that wrap the existing `RelayClient` WebSocket logic. First step: extract framing/auth helpers from `server/src/relay/client.rs` into `transport.rs` so `client.rs` becomes the WebSocket-specific implementation.
- Mirror the trait on the client side inside `client-lib/src/transport/{mod,websocket}.rs` so the CLI and Android clients can swap implementations without touching higher-level enrollment or command logic.
- Start with a minimal trait that maps to the current control flow; add stream helpers later when telemetry/control split is introduced.

```rust
pub trait TransportSession {
    async fn connect(...) -> Result<Self>
    where
        Self: Sized;
    async fn authenticate(&mut self, RegisterPayload) -> Result<RegisterAck>;
    async fn open_stream(&mut self, stream: StreamKind) -> Result<Box<dyn FramedStream>>;
    async fn close(&mut self) -> Result<()>;
}
```
*The initial QUIC implementation can stub `open_stream` to return the control stream until we introduce secondary channels.*

### 4.2 Protocol Negotiation
- Server exposes configuration listing enabled transports and ports.
- Client CLI/lib adds a `--transport=[websocket|quic|auto]` flag; `auto` attempts QUIC first and falls back to WebSocket.
- During handshake, server responds with negotiated transport metadata and capability flags.
- Extend `proto/handcontrol.proto` `RelayInfo` message with `repeated string transports`, `uint32 quic_port`, and `bool quic_preferred`. Regenerate bindings in `client-lib` and server to keep the API backwards-compatible (fields are optional).
- Include the negotiated transport inside the existing `/register` JSON ack (`register_ack`) so the relay control loop can surface fallback reasons without impacting non-QUIC clients.
- Update `server/src/config/parser.rs` to parse new `[relay.transports]` config, mapping into the runtime struct that powers both WebSocket and QUIC listeners.

### 4.3 QUIC Stack Selection
- Evaluate Rust libraries: `quinn` (Rust-native, Tokio-friendly) vs `quiche` (C binding). Favor `quinn` for ecosystem fit unless blockers emerge.
- Decide on deployment model: native UDP listener per server instance, optional SO_REUSEPORT for load distribution.
- Use TLS certificates from existing automation (ACME) or ship self-signed certs for development.
- Spike a `quinn` proof-of-concept in `server/examples/quic_echo.rs` and `client-lib/examples/quic_ping.rs` to validate handshake flow. Document findings inline in this plan (link to snippets).
- Add a `quinn = { version = "...", features = ["runtime-tokio"] }` dependency to `server/Cargo.toml` and `client-lib/Cargo.toml`, guarded by a `quic` Cargo feature so the crate compiles without QUIC enabled.
- Confirm that Android (if applicable) can depend on `quinn` via the shared client lib; otherwise plan to gate QUIC transport from mobile until bindings are ready.

### 4.4 Session & Stream Model
- Map control channel to a reliable bidirectional stream (stream 0).
- Map telemetry or high-frequency data to additional ordered streams; consider QUIC datagrams for best-effort telemetry if latency wins justify complexity.
- Implement keepalive using QUIC ping frames; reuse existing heartbeat payloads.
- Support connection migration by allowing token revalidation when client IP changes.
- Define explicit stream purposes in code (`enum StreamKind { Control, TunnelData, Telemetry }`) and ensure both transports translate them into the appropriate primitives (WebSocket = single multiplexed channel for now, QUIC = real streams).
- Reuse the existing `TunnelReadyMessage`/`OpenTunnel` flow by binding QUIC tunnel streams to the same `tokio::io::split` interface used by WebSocket bridging in `server/src/relay/client.rs`.
- Plan for datagrams by introducing a `TransportCapabilities` struct returned from the trait that indicates `supports_datagrams` and `max_streams` so higher layers can decide when to opt-in.

### 4.5 WebSocket Path Adjustments
- Refactor shared logic (auth, framing, command routing) to use transport abstraction.
- Ensure WebSocket heartbeat timeouts align with QUIC idle timeout settings to keep behavior consistent.
- Maintain current ports and reverse-proxy compatibility for environments that cannot open UDP.
- Move WebSocket-specific request building (headers, TLS override) into `server/src/relay/transport/websocket.rs` to avoid regressing existing behavior during refactor.
- Add regression tests that spin up the in-process WebSocket relay (maybe via `tokio_tungstenite::accept_async`) and reuse them for QUIC by instantiating the trait twice inside the same harness.

### 4.6 Control & Data Plane Messages
- Update `/register` and `/open_tunnel` payloads to include a `transport_session_id` so QUIC migration can re-bind to an existing logical session without replaying registration.
- Introduce a `transport` field on client telemetry events so the observability stack can chart QUIC vs WebSocket metrics without inference.
- Document JSON schema changes in `docs/server-implementation.md` and add examples that show WebSocket vs QUIC handshake payloads side-by-side.

## 5. Implementation Phases
### Phase 0 – Discovery & Design (1-2 weeks)
- Validate library choice (prototype echo server/client, confirm handshake speed, evaluate API).
- Document certificate provisioning workflow and secrets management updates.
- Align stakeholders on Cloudflare limitation; plan alternative exposure (direct UDP or Spectrum).
- Produce a short spike doc summarizing the `quinn` POC, TLS handling, and mobile feasibility; attach traces/metrics to justify the choice.

### Phase 1 – Core Transport Abstraction (1 week)
- Refactor relay core to interact with a transport trait/interface.
- Add unified framing and auth logic independent of transport.
- Ensure WebSocket implementation passes existing integration tests post-refactor.
- Deliverables:
  - `server/src/relay/transport.rs` trait + WebSocket adapter; `RelayClient::connect_and_run` rewritten to depend on the trait.
  - `client-lib/src/transport/mod.rs` with shared auth/framing types used by CLI (`cli/src/main.rs`) and Android terminal integration (`android/`).
  - Regression test `server/tests/relay_websocket_transport.rs` covering registration + tunnel open using the new abstraction.

### Phase 2 – QUIC Server Integration (2 weeks)
- Implement QUIC listener service, connection accept loop, and stream management.
- Hook QUIC sessions into the relay routing layer.
- Expose configuration for QUIC port, TLS cert paths, idle timeout, max streams, congestion tuning.
- Add logging/metrics for handshake duration, RTT, packet loss, migrations, errors.
- Create `server/src/relay/transport/quic.rs` leveraging `quinn::Endpoint`. Integrate with the main relay runtime (likely in `server/src/main.rs`) behind a `RelayTransportKind::Quic` enum.
- Reuse existing JWT token validation by calling into `RelayClient::issue_tunnel_token` prior to admitting a QUIC session.
- Add integration test `server/tests/relay_quic_transport.rs` that spins up a QUIC endpoint and verifies tunnel data can round-trip to a local TCP echo server.

### Phase 3 – Client Integration (2 weeks)
- Extend CLI and client lib to open QUIC connections using the chosen library (e.g., `quinn` client).
- Implement transport negotiation, fallback, and reconnection logic.
- Capture telemetry (latency, throughput) and expose via CLI diagnostics.
- Add `--transport` flag handling in `cli/src/main.rs` and propagate to `handcontrol_client_lib::connect` (new API).
- Update Android terminal integration to read the same config flag, defaulting to `auto`.
- Provide developer documentation (`docs/android-terminal-integration-plan.md` appendix) explaining how the mobile app packages the QUIC runtime.

### Phase 4 – Reliability & Mobility Enhancements (1 week)
- Implement automatic reconnection with session resumption, token replay protection, and path migration support.
- Tune flow control and congestion parameters; simulate lossy network behavior.
- Decide on optional datagram channel for best-effort updates; measure performance impact.
- Store resumption tickets in `client-lib` using the existing config persistence layer; wipe on authentication failure.
- Extend the `TransportSession` trait with a `migrate` method that accepts the new path metadata and rebinds streams.
- Capture packet loss and migration events via structured logs so ops can diff QUIC/WebSocket behavior.

### Phase 5 – Testing & Hardening (2 weeks)
- Unit tests for transport abstraction, handshake, auth, error handling.
- End-to-end tests covering QUIC + WebSocket with shared fixtures.
- Network impairment tests using `tc netem` (packet loss, jitter, latency).
- Scale tests benchmarking CPU/memory under mixed transport load.
- Security review: TLS configuration, certificate rotation, DoS considerations.
- Add a GitHub Actions job (or extend existing CI) that runs QUIC-enabled tests behind a feature flag (`cargo test --features quic`).
- Produce a soak-test script in `scripts/quic_smoke_test.sh` that ops can run manually against staging.

### Phase 6 – Deployment & Rollout (1 week)
- Add feature flags/env vars to enable QUIC per environment.
- Update deployment manifests (ports/firewall rules, health probes).
- Provide operational runbook (monitoring dashboards, alert thresholds, troubleshooting steps).
- Conduct staged rollout: dev → staging → canary → production; observe metrics before expanding exposure.
- Use the existing config hot-reload path to toggle QUIC without restarting services and document the exact YAML/TOML snippet in the runbook.

## 6. Observability & Tooling
- Metrics: handshake duration, active sessions per transport, retransmits, congestion window, migration count.
- Logs: structured events for connect/disconnect, auth failures, transport downgrade, TLS issues.
- Tracing: propagate trace IDs across transport boundary for correlation with backend processing.
- Dashboards comparing QUIC vs WebSocket performance (latency, error rate).

## 7. Documentation & Developer Experience
- Update README/docs to describe transport options, configuration, firewall requirements, and Cloudflare caveats.
- Provide migration guide for operators (enabling UDP ports, cert management).
- Add client usage examples showcasing transport selection and troubleshooting steps.
- Document local development workflow (self-signed cert generation, QUIC debugging tips).

## 8. Risks & Mitigations
- **Firewall/NAT blocking UDP:** Default to WebSocket; surface clear errors and fallback logic.
- **Certificate distribution complexity:** Automate via ACME or integrate with existing secrets tooling; rotate certificates before expiry.
- **Library maturity/bugs:** Track upstream issues, pin versions, and maintain minimal wrapper to swap libraries if required.
- **Operational blind spots:** Ensure metrics and alerts exist before broad rollout; run chaos tests to validate recovery.
- **Cloudflare Tunnel incompatibility:** Offer deployment guidance (direct exposure, Spectrum) and maintain WebSocket path for tunnel users.

## 9. Outstanding Decisions & Questions
- Confirm preferred QUIC library and language bindings for client platforms (desktop, mobile).
- Decide whether to support QUIC datagrams at launch or defer to later iteration.
- Clarify auth requirements for roaming sessions (token reuse vs new issuance).
- Determine minimum supported environments (OS, kernel versions, UDP availability).
- Evaluate need for QUIC-specific rate limiting or admission control to mitigate amplification attacks.
