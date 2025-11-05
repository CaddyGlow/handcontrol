# Project Completion Checklist – Relay-Enhanced QR Enrollment

This checklist captures every remaining task needed to ship the relay-backed enrollment flow across the HandControl stack. Work is grouped by area; treat each checkbox as a concrete deliverable. When all sections are complete, the feature is ready for release.

---

## 1. Server & Relay Hardening
- [ ] Add validation that `relay.include_in_enrollment = true` only succeeds when URL, signing key, and WebSocket subprotocol are configured; emit actionable startup errors.
- [ ] Extend `TokenIssuer` so enrollment tokens embed provisional client identifiers and server audiences; document claim semantics for future auditing.
- [ ] Implement relay token revocation on `handcontrol server clients revoke` (notify relay client / drop cached tunnels).
- [ ] Emit structured telemetry/logs for relay token issuance and consumption (token subject, audience, expiry).
- [ ] Enforce `valid_until` on enrollment payloads and reject stale relay tokens; add regression test.
- [ ] Add server-side config lint that warns when relay is enabled but TLS fingerprints/self-signed settings are inconsistent.
- [ ] Write integration test covering: generate QR → client enrolls via relay → relay token expires → renewal required.

## 2. Rust Clients (CLI, TUI, client-lib)
- [ ] Update CLI help (`handcontrol enroll qr --help`) to document relay fallback, new TSV columns, and JSON fields.
- [ ] In TUI, surface connection preference (auto/direct-only/relay-only) and add toggle UI for each server.
- [ ] Add client-lib unit tests for relay fallback ordering (`Attempt::Relay` cases, pinned fingerprint with relay).
- [ ] Provide example config snippet in README showing how to enable relay + preferences.
- [ ] Cut a workspace release once tests pass (`cargo test --workspace`, `cargo fmt --all --check`).
- [ ] Implement error messaging and retry prompts for relay handshake failures (expired token, TLS mismatch, misconfiguration).
- [ ] Ensure client-lib stores relay credentials per server and keeps direct→relay fallback ordering deterministic.

## 3. Android Client
- [ ] Implement WorkManager job that refreshes relay tokens alongside certificate renewal; expose refresh interval and back-off strategy.
- [ ] Show relay diagnostics in server detail view (latency, success rate, last relay error).
- [ ] Add connection statistics screen (counts of direct vs relay sessions, last failure cause).
- [ ] Write unit tests for new settings (default connection preference) and ServerConnectionManager fallback respecting user preference.
- [ ] Add instrumentation tests: direct success, forced relay, relay-only failure, token expiry path.
- [ ] Create manual QA script: same-network, cross-network, relay disabled, preference overrides.
- [ ] Surface explicit user-facing errors for relay TLS mismatch or missing fingerprint, with remediation guidance.

## 4. Documentation & Operator Workflow
- [ ] Update README with "Enrolling Across Networks" quick-start (server + CLI/TUI + Android).
- [ ] Expand `docs/SECURITY.md` with relay enrollment threat model and incident response steps.
- [ ] Document relay deployment (TLS guidance, token TTL tuning, key rotation) in `docs/FEATURE_RELAY.md`.
- [ ] Provide troubleshooting guide (common errors, log locations, metrics) in `docs/PROJECT_STRUCTURE.md` or dedicated doc.
- [ ] Publish operator runbook for rollout/migration (enabling flag, validating relay health, rollback steps).

## 5. QA, Release & Monitoring
- [ ] Run full automated test matrix: `cargo test --workspace`, Android unit tests (`./gradlew :app:testDebugUnitTest`), lint (`./gradlew :app:lint`), relay integration tests.
- [ ] Perform manual end-to-end verification: generate QR, enroll CLI, enroll Android (same/different network), execute commands, verify relay fallback.
- [ ] Prepare release notes summarizing new enrollment experience, preferences, and relay requirements.
- [ ] Instrument monitoring/alerts: relay usage counts, failure rates, token expiration warnings.
- [ ] Tag release (git tag, crates.io publish if applicable) and update changelog.
- [ ] Capture baseline metrics before enabling relay enrollment to establish success benchmarks.

## 6. Migration & Risk Management
- [ ] Coordinate client release timelines so relay-aware builds are available before operators enable relay-only enrollment.
- [ ] Rotate relay signing keys in staging to validate dual-key window and rollback procedures.
- [ ] Validate HA/high-availability posture for relay infrastructure (failover test, monitoring alerts).
- [ ] Review analytics pipeline to ensure relay usage metrics remain privacy-safe (counts/durations only).

---

**Completion Criterion:** Every checkbox checked, automated tests green, manual QA sign-off recorded, and documentation merged. At that point the relay-enhanced enrollment feature is production-ready.
