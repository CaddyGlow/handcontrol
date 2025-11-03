# Relay End-to-End Encryption Plan

## Goal
Ensure the relay can forward gRPC traffic between mobile clients and managed servers without ever being able to decrypt or tamper with the payload. After this change:

- TLS terminates **only** at the client app and the target server.
- The relay only observes opaque TLS records and performs byte forwarding.
- Existing authentication (JWTs, relay secrets) continues to protect tunnel setup.

## Non-Goals
- Replacing the current relay authentication flow.
- Supporting legacy clients that cannot establish TLS directly to the server.
- Introducing dynamic certificate issuance; we assume the server already has a valid certificate/key pair and the client can obtain the corresponding trust anchors.

## High-Level Architecture Changes
1. **Client (Android)**
   - Replace `OkHttpChannelBuilder.usePlaintext()` with an mTLS configuration so the app negotiates TLS directly with the server.
   - Reuse the existing `MtlsGrpcChannelFactory` flow (or extract its logic) so relay-backed channels share certificate pinning, trust capturing, and lifecycle handling with direct connections.
   - Preserve SNI/authority when tunnelling by targeting the relay-provided hostname and installing a custom resolver that returns `127.0.0.1`/`::1`, so TLS validation still binds to the real server name while sockets dial the local bridge.
   - Reuse the local TCP bridge but run TLS over it; the relay must treat the bridge as opaque bytes.

2. **Relay Server (Rust)**
   - Ensure tunnel forwarding never interprets or modifies the payload beyond WebSocket framing (already mostly true).
   - Confirm that the relay does not buffer or log plaintext messages; update telemetry to avoid dumping binary contents.

3. **Managed Server Relay Client (Rust)**
   - Remove the local TLS termination that currently wraps the gRPC connection; when acting as a relay client, treat byte streams as opaque and forward them unchanged so the server-side gRPC endpoint performs the TLS handshake with the Android client.
   - Audit configuration to ensure any server-side certificate pinning or validation remains compatible once TLS terminates only on the managed server.

4. **Configuration / Certificate Distribution**
   - Document how the Android client obtains the server’s TLS authority (currently supplied via `connect_ack.server_authority`).
   - Clarify fallback behavior if the relay cannot provide a custom authority (default host:port).

## Detailed Implementation Steps

### 1. Android Client Updates
1. **Refactor `RelayGrpcChannelFactory`**
   - Refactor the factory to delegate to `MtlsGrpcChannelFactory` (or a shared helper) for TLS configuration so the relay path inherits the same certificate capture/pinning logic as direct connections.
   - When starting the local bridge, still listen on a loopback TCP port, but wrap the resulting gRPC channel in mTLS (`sslSocketFactory`, pinned fingerprint, client cert).
   - Ensure certificate pinning aligns with whatever the server provides (trust the TLS authority from `connect_ack` when present).

2. **Connection Lifecycle**
   - Verify the new TLS setup works with the caching logic we recently added. Double-check that shutdown closes TLS cleanly.

3. **Error Handling**
   - Surface meaningful errors if TLS negotiation fails (mismatched fingerprints, expired certs).

### 2. Relay Backend Verification
1. **Code Audit**
   - Review `forward_stream` in `relay/src/main.rs`; confirm it just forwards binary frames and does not attempt to interpret data.
   - Remove/guard any logging that might print raw binary payloads (currently traces only print sizes; keep it that way).

2. **Testing**
   - Extend integration tests to perform a TLS handshake over the tunnel:
     - Android-side tests are hard to automate but we can create a Rust integration test using tonic with TLS enabled.
     - Verify relay still handles timeouts, close frames, and ping/pong without peeking into TLS contents.

### 3. Managed Server Relay Client
1. **Compatibility Check**
   - Update `handle_tunnel` so it no longer establishes a local TLS session; instead, forward the raw TCP stream to the server’s gRPC listener and let that endpoint complete the mTLS handshake with the Android client.
   - Confirm certificate pinning logic (if any) is still correct once TLS negotiation happens end-to-end between client and server.

### 4. Configuration & Documentation
1. **Docs**
   - Update relay configuration guides to emphasize that the server certificate presented to clients must be trusted/pinned.
   - Document fallback behavior when `server_authority` is omitted.

2. **Migration Notes**
   - Spell out any client/server version requirements (e.g., clients must include the new TLS-supporting build, servers must provide TLS endpoints).

## Testing Strategy
- **Unit Tests**: Validate new Android TLS channel creation paths.
- **Integration Tests**: Use the Rust test suite to establish a TLS gRPC session through the relay.
- **Manual QA**: Run the Android app against a staging relay to confirm end-to-end encryption, observing that the relay logs only show byte counts.

## Rollout / Deployment
1. Deploy updated relay backend (no breaking change expected).
2. Roll out server update if any logging tweaks are made.
3. Release updated Android client; ensure feature flag / configuration gating if necessary.
4. Monitor for TLS handshake failures or unexpected timeouts.
