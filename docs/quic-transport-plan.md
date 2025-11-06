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

### 4.2 Protocol Negotiation
- Server exposes configuration listing enabled transports and ports.
- Client CLI/lib adds a `--transport=[websocket|quic|auto]` flag; `auto` attempts QUIC first and falls back to WebSocket.
- During handshake, server responds with negotiated transport metadata and capability flags.

### 4.3 QUIC Stack Selection
- Evaluate Rust libraries: `quinn` (Rust-native, Tokio-friendly) vs `quiche` (C binding). Favor `quinn` for ecosystem fit unless blockers emerge.
- Decide on deployment model: native UDP listener per server instance, optional SO_REUSEPORT for load distribution.
- Use TLS certificates from existing automation (ACME) or ship self-signed certs for development.

### 4.4 Session & Stream Model
- Map control channel to a reliable bidirectional stream (stream 0).
- Map telemetry or high-frequency data to additional ordered streams; consider QUIC datagrams for best-effort telemetry if latency wins justify complexity.
- Implement keepalive using QUIC ping frames; reuse existing heartbeat payloads.
- Support connection migration by allowing token revalidation when client IP changes.

### 4.5 WebSocket Path Adjustments
- Refactor shared logic (auth, framing, command routing) to use transport abstraction.
- Ensure WebSocket heartbeat timeouts align with QUIC idle timeout settings to keep behavior consistent.
- Maintain current ports and reverse-proxy compatibility for environments that cannot open UDP.

## 5. Implementation Phases
### Phase 0 – Discovery & Design (1-2 weeks)
- Validate library choice (prototype echo server/client, confirm handshake speed, evaluate API).
- Document certificate provisioning workflow and secrets management updates.
- Align stakeholders on Cloudflare limitation; plan alternative exposure (direct UDP or Spectrum).

### Phase 1 – Core Transport Abstraction (1 week)
- Refactor relay core to interact with a transport trait/interface.
- Add unified framing and auth logic independent of transport.
- Ensure WebSocket implementation passes existing integration tests post-refactor.

### Phase 2 – QUIC Server Integration (2 weeks)
- Implement QUIC listener service, connection accept loop, and stream management.
- Hook QUIC sessions into the relay routing layer.
- Expose configuration for QUIC port, TLS cert paths, idle timeout, max streams, congestion tuning.
- Add logging/metrics for handshake duration, RTT, packet loss, migrations, errors.

### Phase 3 – Client Integration (2 weeks)
- Extend CLI and client lib to open QUIC connections using the chosen library (e.g., `quinn` client).
- Implement transport negotiation, fallback, and reconnection logic.
- Capture telemetry (latency, throughput) and expose via CLI diagnostics.

### Phase 4 – Reliability & Mobility Enhancements (1 week)
- Implement automatic reconnection with session resumption, token replay protection, and path migration support.
- Tune flow control and congestion parameters; simulate lossy network behavior.
- Decide on optional datagram channel for best-effort updates; measure performance impact.

### Phase 5 – Testing & Hardening (2 weeks)
- Unit tests for transport abstraction, handshake, auth, error handling.
- End-to-end tests covering QUIC + WebSocket with shared fixtures.
- Network impairment tests using `tc netem` (packet loss, jitter, latency).
- Scale tests benchmarking CPU/memory under mixed transport load.
- Security review: TLS configuration, certificate rotation, DoS considerations.

### Phase 6 – Deployment & Rollout (1 week)
- Add feature flags/env vars to enable QUIC per environment.
- Update deployment manifests (ports/firewall rules, health probes).
- Provide operational runbook (monitoring dashboards, alert thresholds, troubleshooting steps).
- Conduct staged rollout: dev → staging → canary → production; observe metrics before expanding exposure.

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
