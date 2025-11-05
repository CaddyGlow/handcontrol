# Capability-Oriented Refactor

## Objectives
- Replace the legacy shell-command execution path with a capability-centric architecture that supports one-shot actions and realtime sessions (interactive shell, future mouse control, file transfer, etc.).
- Cleanly separate configuration/discovery concerns from session runtime management to improve extensibility, policy enforcement, and observability.
- Deliver a secure, low-latency interactive shell capability as the first realtime feature, providing the foundation for additional high-interactivity capabilities.

## Architecture Refactor Plan

### 1. Domain Model & Registry
- Define core domain types: `CapabilityId`, `CapabilityMetadata` (name, tags, description, session mode, parameter schema), `SessionMode` (`OneShot`, `Realtime`, `Upload`, `Download`), `CapabilityKind` (shell, shell_interactive, file_transfer, etc.).
- Introduce a `CapabilityRegistry` responsible for:
  - Loading capability definitions from config.
  - Enforcing uniqueness of capability ids.
  - Exposing immutable handles to capability implementations.
  - Providing metadata for discovery APIs.
- Refactor existing shell command execution into a `ShellScriptCapability` that implements a `Capability` trait (e.g., `fn open(&self, ctx) -> CapabilitySession`).

### 2. Configuration Overhaul
- Replace the current `command` array in config with a `capabilities` section containing typed entries (serde tagged enum or similar):
  ```toml
  [[capabilities]]
  id = "shell.dev.build"
  type = "shell_script"
  display_name = "Build Project"
  session_mode = "one_shot"
  shell.exec = "cargo build --release"
  shell.timeout_seconds = 600
  acl.allow = ["group:admins", "fingerprint:abc123"]
  ```
- Define shared schema pieces: parameter definitions, confirmation flags, per-capability ACL requirements, session timeouts.
- Add validation to ensure config references only supported capability types, required fields are present, and ACL rules are consistent.
- Update defaults, config watcher, and documentation to reflect the new schema; provide migration notes for existing deployments.

### 3. Control Plane Services
- Create a `CapabilityService` module that wraps the registry, providing:
  - `list_capabilities()` for `ListCapabilities` RPC.
  - `authorize(capability_id, client_ctx)` to check ACLs before session creation.
  - Change notifications when config reloads update capabilities (tie into current broadcaster).
- Ensure capability metadata returned to clients includes versioning (config version) for cache consistency.

### 4. Session Runtime & Execution Plane
- Implement a `SessionManager` responsible for:
  - Creating sessions via capability implementations.
  - Managing session lifecycles (start, heartbeat, timeout, termination).
  - Tracking active sessions in a concurrent map keyed by `SessionId`.
  - Publishing observability data (metrics, tracing spans).
- Define `CapabilitySession` variants (`OneShotHandle`, `RealtimeHandle`, etc.) encapsulating channel endpoints for gRPC bridging.
- Standardize telemetry structs (bytes sent/received, duration, exit status) for auditing and monitoring.

### 5. gRPC API Redesign
- Deprecate `ListCommands` / `ExecuteCommand`; introduce:
  - `ListCapabilities`: unary RPC returning capability metadata and config version.
  - `OpenSession`: bidirectional streaming RPC using `SessionMessage` envelopes:
    ```proto
    message SessionOpen { string capability_id; map<string,string> parameters = 2; }
    message SessionInput { bytes data = 1; optional ShellResize resize = 2; }
    message SessionOutput { bytes data = 1; bool is_stderr = 2; }
    message SessionHeartbeat { int64 latency_hint_ms = 1; }
    // Wrapped in oneof on both client+server messages.
    ```
- Include session lifecycle messages (`SessionReady`, `SessionClosed`, `SessionError`) so clients can handle state transitions robustly.
- Update middleware (auth, telemetry) to understand session streams.

### 6. Security & Policy Foundation
- Extend existing TLS fingerprint checks with per-capability ACL evaluation; integrate with config-defined allow/deny lists and optional group mapping (e.g., from certificate metadata).
- Run sensitive capabilities (interactive shell) under constrained service accounts; support platform-specific sandboxing hooks.
- Implement idle timeout and absolute lifetime caps per capability; surface events in logs and metrics.
- Add structured audit logging for session start/stop, ACL denials, and errors (timestamp, client fingerprint, capability id, exit status).

### 7. Observability Enhancements
- Instrument capability/session operations with tracing spans and metrics:
  - Active sessions per capability.
  - Session duration histograms.
  - Bytes transferred upstream/downstream.
  - Latency measurements between input and output messages.
- Integrate session events with existing broadcaster/notification system if operators need real-time alerts for high-privilege capability usage.

### 8. Documentation & Tooling
- Update README/docs to explain capability model, new config schema, gRPC API, and security considerations.
- Refresh CLI tooling (if any) to render capability metadata and initiate sessions.
- Provide migration script or manual steps to convert old configs to the new format.

## Implementation Plan (Detailed)

### Phase 1: Protobuf & Client Foundations
- Draft the new protobuf definitions for capabilities and sessions; review field names/types for forward compatibility.
- Regenerate Rust (and other language) bindings; introduce feature flags in client libraries to toggle between legacy and new RPCs during development.
- Update proto documentation to describe session message flow and error semantics.

### Phase 2: Capability Registry & Config Parser
- Implement new config data structures and parsing logic, including validation and tests for representative examples.
- Build `CapabilityRegistry` with unit tests covering duplicate IDs, unknown types, ACL parsing, and config reload updates.
- Wrap the existing shell command executor in a `ShellScriptCapability` to validate the trait design (still using the old RPC temporarily).

### Phase 3: SessionManager Infrastructure
- Design `SessionId` generation, session state structs, and concurrency primitives (Tokio channels, dashmap/RwLock).
- Implement mock capability handlers (e.g., echo capability) to test session lifecycle without OS dependencies.
- Add heartbeats, idle timeout handling, and termination logic; cover with integration-style async tests.

### Phase 4: gRPC Service Migration
- Implement `ListCapabilities` and `OpenSession` RPCs using the new services while keeping legacy RPCs available behind feature flag.
- Bridge session channels to gRPC streams (spawn tasks to forward messages, handle errors, propagate closures).
- Add authentication + ACL middleware that retrieves the client context (fingerprint, metadata) and validates capability access before opening sessions.

### Phase 5: Interactive Shell Capability
- Build PTY abstraction layer (`pty::unix`, `pty::windows`) returning async read/write handles and resize controls.
- Implement `ShellInteractiveCapability`:
  - Spawn subprocess in PTY with user-configured shell and environment.
  - Pipe PTY output into session outbound channel with timestamping.
  - Handle input writes, control signals (Ctrl+C), and resize messages.
  - Enforce per-session timeouts and exit status reporting.
- Write end-to-end tests on Linux (using integration harness) and targeted unit tests with PTY fakes/mocks.

### Phase 6: Security Hardening
- Enforce ACLs in `CapabilityService::authorize`, with tests for allow/deny permutations and error propagation to gRPC layer.
- Wire interactive shell to run under service account/sandbox if configured; document setup steps for each platform.
- Implement audit logging hooks and ensure telemetry includes capability id, client fingerprint, session id, and outcome.

### Phase 7: Cleanup & Migration
- Remove legacy `commands` module, `ExecuteCommand` RPC, and config structures once new path is verified.
- Update documentation, CLI tools, and example configs to use the capability model exclusively.
- Provide migration guidance (sample conversion script, manual checklist).

### Phase 8: Stabilization & Future Work
- Conduct performance tuning for session latency (channel sizing, TCP_NODELAY, batching heuristics).
- Monitor metrics in staging; adjust timeouts and backpressure as needed.
- Queue follow-up capabilities (mouse control, keyboard macros, file transfer) now that the foundation is in place.

