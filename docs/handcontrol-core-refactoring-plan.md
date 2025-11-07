# Handcontrol Core Extraction & P2P Ramp Plan (Solo-Friendly)

**Status**: Work in progress
**Owner**: Core maintainer (single-dev bandwidth)
**Last Updated**: 2025-11-07 (reviewed and corrected)
**Depends On**: `docs/p2p-implementation-plan.md`, existing QUIC experiments
**Repositories / Crates**: `client-lib`, `server`, `relay`, `cli`, `tui`, `proto`, `handcontrol-core` (new)

---

## TL;DR
- Create a small `handcontrol-core` crate that holds code already duplicated across `client-lib`, `server`, and `relay`.
- Move code in thin, testable slices so one person can land each milestone in sequence without blocking daily fixes.
- Keep the current crate names (`client-lib`, `cli`, `tui`) to avoid churn; the rename to `handcontrol-client` stays optional backlog.
- Align every extraction with workspace dependencies already in use (quinn, rustls, etc. via `workspace.dependencies`).
- Defer P2P features until the core crate is battle-tested, then layer the P2P timeline on top of the cleaned-up APIs.

---

## Operating Constraints
1. **Single Maintainer** – only one developer is actively moving this plan, so each chunk must take ≤3 working days including review.
2. **No Feature Freeze** – CLI/TUI/server must keep compiling; `client-lib` remains the dependency name for public crates.
3. **CI Guardrails** – treat `cargo fmt`, `cargo clippy --workspace`, and `cargo test --workspace` as required before/after every milestone.
4. **Version Discipline** – new crates inherit dependency versions from `[workspace.dependencies]`; no downgrades or duplicate semver lines.
5. **Clean Slate Allowed** – we can drop backward compatibility when it simplifies the stack; prioritize the new protocol surface rather than maintaining fallbacks.

---

## Current Snapshot

### Workspace & Structure
- Workspace members today: `server`, `relay`, `client-lib`, `cli`, `tui`. (`proto/handcontrol.proto` is not a Cargo crate and must **not** be listed as a workspace member.)
- `cli` and `tui` both depend on `handcontrol-client-lib = { path = "../client-lib" }`; breaking that path would block both binaries.
- Tests: `cargo test --workspace` currently covers QUIC, WebSocket, CLI command paths, and relay control flow; there are no integration tests for Android yet.

### Duplication Hotspots (quick inventory)

| Area | Files | LOC (approx) | Notes |
|------|-------|--------------|-------|
| Frame encoding/decoding | `client-lib/src/transport/quic.rs`, `relay/src/main.rs` (QUIC helpers), `server/src/relay/transport/quic.rs` | ~100 | Identical constants and read/write helpers. |
| TLS self-signed verifier | `client-lib/src/transport/quic.rs`, `client-lib/src/transport/websocket.rs`, `server/src/relay/transport/quic.rs` | ~80 | Same fingerprint logic, only struct names differ. |
| File/dir permissions | `client-lib/src/certificates.rs`, `server/src/security/certificates.rs` | ~30 | Straightforward `cfg(unix)` helpers. |
| Protocol control messages | `client-lib/src/relay.rs`, `server/src/relay/mod.rs`, `relay/src/main.rs` | ~120 | Serialization drift + no versioning. |
| Config helpers | `client-lib/src/config.rs`, `server/src/config/parser.rs` | ~100 | Shared env overrides + defaults. |
| QUIC transport plumbing | `client-lib/src/transport/quic.rs`, `server/src/relay/transport/quic.rs` | ~200 | Endpoint creation, ALPN, forwarding. |

These blocks alone cover ~550–600 LOC of obvious duplication that can be moved without inventing new abstractions.

---

## Roadmap at a Glance

| Milestone | Duration (target) | Focus | Key Exit Criteria |
|-----------|-------------------|-------|-------------------|
| **M0 – Baseline & Safety Net** | 0.5 week | Tests, metrics, checklist | Fresh `cargo test --workspace`, dependency audit clean, CI green. |
| **M1 – Core Skeleton** | 1 week | Create crate, move perms + TLS + frames | `handcontrol-core` compiled, consumers switched, zero behaviour change. |
| **M2 – Certificates & Config** | 1 week | Fingerprints + config helpers + env overrides | Shared API covers existing env vars, CLI/TUI unaffected. |
| **M3 – Protocol & Messaging** | 1 week | Control messages, serialization helpers, versioning | Relay/client share message structs, envelope parser validated. |
| **M4 – Transport Utilities** | 1.5 weeks | QUIC setup, frame forwarding, connection states | QUIC code in client/server shrinks, forwarding smoke tests pass. |
| **M5 – Relay Migration & Hardening** | 1 week | Relay adopts core crate, regression tests | Relay runs solely on the new core APIs, manual QA pass. |
| **P2P Ramp (Post-refactor)** | 4–6 weeks | ICE, P2P transports, rollout | Builds on new core APIs with feature flags. |

Each milestone ends with a mergeable PR and a tagged checklist entry in `cli_refactor_progress.txt` (or a new tracking doc).

---

## Milestone Details

### M0 – Baseline & Safety Net
**Goals**
- Capture the current behaviour so regressions are easy to spot.
- Ensure CI scripts are repeatable locally.
- Audit dependencies for security vulnerabilities.

**Tasks**
1. Run `cargo fmt`, `cargo clippy --workspace --all-targets`, and `cargo test --workspace`.
2. Run dependency audit: `cargo audit` and `cargo outdated --workspace`.
3. Document active env variables in `client-lib/src/config.rs` (e.g., `HANDCONTROL_CONFIG_DIR`, `HANDCONTROL_TRANSPORT`).
4. Add a short "Refactor tracker" section to `docs/p2p-progress.md` or keep `cli_refactor_progress.txt` updated.

**Exit Criteria**
- All tests pass twice in a row.
- Environment-variable inventory captured in docs (new section or file committed).
- Known flaky tests logged with issue links.
- Dependency audit shows no critical vulnerabilities or action plan documented.

### M1 – Core Skeleton (Perms, TLS, Frame Format)
**Goals**: Introduce the crate with minimal API: file permissions, TLS verifier, synchronous frame format definitions.

**Tasks**
1. `cargo new --lib handcontrol-core`; add it to the end of `[workspace.members]`:
   ```toml
   members = ["server", "relay", "client-lib", "cli", "tui", "handcontrol-core"]
   ```
2. In `handcontrol-core/Cargo.toml`, use workspace dependencies instead of pinned versions:
   ```toml
   [dependencies]
   anyhow = { workspace = true }
   tokio = { workspace = true }
   quinn = { workspace = true }
   rustls = { workspace = true }
   sha2 = { workspace = true }
   ```
3. Move file-permission helpers into `handcontrol_core::io::file_permissions`; export via `src/lib.rs`.
4. Consolidate the self-signed verifier into `handcontrol_core::security::tls_verifier`; keep the fingerprint parameter optional to match current behaviour.
5. Extract frame format definitions into `handcontrol_core::transport::frames`:
   - Frame type constants and enums
   - Frame header layout (`FRAME_HEADER_LEN`)
   - Synchronous encode/decode helpers for frame structure
   - **Note**: Async I/O wrappers (`read_frame`, `write_frame`) will be added in M4
6. Update `client-lib`, `server`, and `relay` to import these modules; update `cli`/`tui` indirectly via `client-lib`.
7. Run unit tests for touched crates plus `cargo test --workspace`.

**Exit Criteria**
- Core crate builds on its own.
- No duplicated code remains for permissions/TLS/frame format.
- Async frame I/O helpers remain in original locations (will move in M4).
- Commit message suggestion: `refactor: add handcontrol-core foundation`.

### M2 – Certificates & Config
**Goals**: Share certificate fingerprint utilities and config-loading helpers without regressing env overrides.

**Tasks**
1. Create `handcontrol_core::security::certificates` with `compute_fingerprint`, `fingerprint_to_hex`, and `parse_fingerprint_hex`.
2. Extract config helpers into `handcontrol_core::config`:
   - `config_dir(app_name)` respecting `HANDCONTROL_CONFIG_DIR`.
   - `load_config<T>()` that reads TOML, supports `env_prefix`, and applies overrides for existing keys (re-use the logic around `TRANSPORT_OVERRIDE_ENV`).
   - Default-value helpers (e.g., `default_timeout_seconds`).
3. Switch `client-lib` and `server` to consume the shared helpers; ensure CLI flags/env vars still take precedence.
4. Add unit tests for parsing env overrides in the core crate:
   - Add `serial_test = { workspace = true }` to core's dev-dependencies
   - Add `serial_test` to workspace.dependencies in root Cargo.toml
   - Use `#[serial]` attribute for env-mutating tests or use `tempfile` for file-based tests

**Exit Criteria**
- Config tests pass on Linux and macOS CI.
- Manual verification: CLI still respects `HANDCONTROL_TRANSPORT`, server still respects `HANDCONTROL_SERVER_CONFIG`.
- All documented env vars verified working in at least one end-to-end test.
- Commit suggestion: `refactor: share cert + config helpers via core`.

### M3 – Protocol & Messaging
**Goals**: Define shared control-message structs with extensible capability-based versioning.

**Tasks**
1. Add `handcontrol_core::protocol::relay` module:
   - `MessageEnvelope { capabilities, payload }` with capability flags (e.g., `SUPPORTS_P2P`, `SUPPORTS_QUIC`)
   - `ControlMessage` enum covering today's relay control flow (Pair, Unpair, Ping, Data, etc.)
   - `ProtocolCapabilities` bitflags for extensibility without hard version bumps
   - Helper: `fn negotiate_capabilities(client: u32, server: u32) -> u32` for feature intersection
2. Provide helpers: `fn encode_message(msg: &ControlMessage) -> Result<String>` and `fn decode_message(raw: &str) -> Result<(ProtocolCapabilities, ControlMessage)>`
3. Update `client-lib/src/relay.rs`, `server/src/relay/mod.rs`, and `relay/src/main.rs` to consume these types.
4. Add serialization round-trip tests in the core crate plus integration test under `relay/tests/protocol_roundtrip.rs`:
   - Spawn minimal relay + mock client
   - Test Pair/Unpair/Ping/Data envelope serialization
   - Validate capability flags preserved and negotiated correctly

**Exit Criteria**
- Client, server, and relay all use the same message structures and capability negotiation.
- Metrics/logging show capability flags for observability.
- Integration test passes with mock relay/client handshake.
- Commit suggestion: `refactor: unify relay protocol types with capabilities`.

### M4 – Transport Utilities
**Goals**: Share QUIC endpoint setup, async frame I/O, and connection state so that client/server QUIC files shrink.

**Tasks**
1. Introduce `handcontrol_core::transport::quic` with:
   - `build_client_endpoint(bind_addr, verifier)` returning `(Endpoint, ClientConfig)`
   - `build_server_endpoint(bind_addr, cert_chain, key)`
   - Shared ALPN constants
   - Async frame I/O helpers: `read_frame`, `write_frame` (moved from M1 extraction scope)
   - **Note**: Validate compatibility with workspace-locked quinn version during migration
2. Add `handcontrol_core::transport::forwarding` with frame forwarding logic:
   - Use the synchronous frame encoding from M1 (`frames` module)
   - Async helpers for `spawn_uplink/downlink` tunnel management
   - Auto-response handlers for Ping/Pong frames
3. Extract lightweight "connection context" structs (ids, metrics, cancellation tokens) to reduce copy/paste in client/server.
4. Update `client-lib/src/transport/quic.rs` and `server/src/relay/transport/quic.rs` to use the shared pieces; keep WebSocket transport untouched for now.
5. Smoke-test QUIC flows using existing examples plus new integration test under `server/tests/quic_forwarding.rs`:
   - Create QUIC server endpoint
   - Send frames with tunnel IDs
   - Verify Ping auto-response
   - Validate tunnel ID logging

**Exit Criteria**
- QUIC transport code in client/server is primarily business logic, not plumbing.
- Async `read_frame`/`write_frame` removed from client-lib and server (now in core).
- Forwarding helpers auto-respond to Ping/Pong and surface tunnel IDs in logs.
- Integration test passes with frame roundtrip validation.
- Commit suggestion: `refactor: share QUIC + forwarding helpers`.

### M5 – Relay Migration & Hardening
**Goals**: Move the relay to `handcontrol-core`, verify behaviour on the new protocol surface, and tidy documentation.

**Tasks**
1. Update `relay/Cargo.toml` to depend on `handcontrol-core`.
2. Replace local helpers with imports from the core crate (frames, protocol, TLS, config).
3. Add end-to-end tests covering the new control-message envelope and QUIC helpers.
4. Run smoke tests for CLI enrollment, server relay workflows, and QUIC tunnels (manual or scripted).
5. Manual verification of all documented env vars (from M0 baseline) still work in relay:
   - Test `HANDCONTROL_CONFIG_DIR` override
   - Test any relay-specific env overrides
6. Update docs (`README.md`, `docs/p2p-progress.md`) to reflect the new architecture.

**Exit Criteria**
- Relay runs entirely on the shared abstractions without referencing legacy code paths.
- All env vars from M0 documentation verified working in relay.
- Smoke tests pass (CLI enrollment, relay tunnel establishment, QUIC forwarding).
- Documentation + diagrams updated.
- Commit suggestion: `refactor: migrate relay to handcontrol-core`.

---

## P2P Ramp (Post-refactor)

Once M1–M5 are merged, follow the `docs/p2p-implementation-plan.md` milestones layered on the new APIs:
1. **P2P Phase A (Weeks 1–2)** – Add optional `p2p` feature flag to `handcontrol-core`, define ICE types, and surface config toggles.
2. **Phase B (Weeks 3–4)** – Implement P2P signaling messages using the capability-based envelope from M3. Signaling will use the existing relay control channel (WebSocket/QUIC transport), not a separate signaling server.
3. **Phase C (Weeks 5–6)** – Build `client-lib` P2P QUIC transport reusing `handcontrol_core::transport::quic`.
4. **Phase D (Weeks 7–8)** – Add server-side listeners + relay brokering.
5. **Phase E (Weeks 9–11)** – Testing, resilience, staged rollout with feature flags.

**Important**: Each P2P phase should only start after the previous refactor milestone has shipped to avoid context thrash. The relay will serve dual purposes:
- **Signaling coordinator** for P2P connection establishment (ICE candidates, offers/answers)
- **Transport fallback** when P2P negotiation fails

---

## Testing & Tooling Strategy
- **Unit Tests**: Every new module in `handcontrol-core` gets dedicated unit tests (frame parsing, env overrides, protocol serde).
- **Integration Tests**: Add lightweight tests under `relay/tests` and `server/tests` to cover handshake flows.
- **Smoke Tests**: Keep `examples/quic_ping.rs` (or similar) up to date; run it after M4 and beyond.
- **Linting**: Enforce `cargo fmt` + `cargo clippy --workspace --all-targets` via CI and before pushing.

---

## Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Scope creep (rename, Android, etc.) | Delays core refactor | Keep backlog section; only one milestone in flight. |
| Forgotten env overrides | Broken operator workflows | Dedicated tests in `handcontrol-core::config`; verify CLI/TUI manually. |
| Divergent dependency versions | Build failures | Always pull from `[workspace.dependencies]`. |
| Regression hard to detect | Users hit bugs first | Baseline metrics + compatibility tests each milestone. |
| Solo maintainer burnout | Schedule slips | Keep milestones ≤3 days, record blockers early. |

---

## Backlog / Nice-to-Have Items
1. Rename `client-lib` → `handcontrol-client` once downstream crates (CLI, TUI, Android) can absorb the change.
2. Add Windows/macOS CI runners for cross-platform file-permission tests.
3. Introduce `xtask` commands for repetitive smoke tests (`cargo xtask smoke-quic`).
4. Expand documentation with Mermaid diagrams once the structure stabilizes.

---

## Appendix A – Proposed Core Layout
```
handcontrol-core/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── config/mod.rs
    ├── io/file_permissions.rs
    ├── protocol/relay.rs
    ├── security/
    │   ├── certificates.rs
    │   └── tls_verifier.rs
    └── transport/
        ├── frames.rs
        ├── forwarding.rs
        └── quic.rs
```

---

## Appendix B – Handy Commands
```bash
# Run once per milestone
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Targeted tests
cargo test -p handcontrol-core
cargo test -p relay relay_protocol_roundtrip

```
