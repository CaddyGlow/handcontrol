# HandControl Android Implementation Guide

## Overview

- Purpose: translate the product requirements (`docs/PRD.md`) into actionable Android workstreams.
- Scope: Kotlin client, Jetpack Compose UI, gRPC/mTLS stack, enrollment flows, command execution.
- Status: Draft; update alongside feature delivery.

## Current Progress

- Gradle build now configured with Hilt, protobuf/gRPC plugins, CameraX, ML Kit, and Timber dependencies (`android/app/build.gradle.kts`).
- Application skeleton in place with Hilt entry point, navigation host, and welcome feature scaffolding (`android/app/src/main/kotlin/com/handcontrol`).
- Core/data interface stubs landed for certificate management, gRPC channels, NSD discovery, and enrollment repositories to anchor upcoming implementations.

## Architecture & Modules

### Layering

- `core`: shared utilities (TLS, keystore access, logging, configuration).
- `data`: protobuf stubs, gRPC clients, repositories, persistence.
- `feature/*`: Compose UI + viewmodels per feature (`enrollment`, `commands`, `settings`).
- `app`: navigation host, dependency injection entry points.

### Technology Selections

- DI: Hilt (aligns with lifecycle awareness and tooling support).
- Async: Kotlin coroutines + Flow.
- Serialization: Protobuf (RPC payloads), Kotlin serialization (local state).
- Networking: gRPC-Kotlin + OkHttp (TLS channel).
- Discovery: Android NSD; fallback manual entry.
- QR: ML Kit Barcode Scanning.
- Logging: Timber (debug builds) with on-device log capture; no remote telemetry.

## Tooling & Build Setup

1. Protobuf tooling in place via `com.google.protobuf` Gradle plugin; ensure shared `.proto` files land under `app/src/main/proto`.
2. Dependencies:
   - `io.grpc:grpc-okhttp`, `io.grpc:grpc-kotlin-stub`, `io.grpc:grpc-protobuf-lite`.
   - `androidx.security:security-crypto` for keystore helpers.
   - `androidx.lifecycle`, `androidx.navigation`, `androidx.datastore`.
   - ML Kit QR, NSD wrappers, logging (Timber-only setup, optional bug report export).
3. Compose BOM: keep repository baseline (`androidx.compose:compose-bom:2025.10.01`).
4. Enable Kotlin KSP (if needed) and configure Hilt Gradle plugins.
5. CI tasks: `./gradlew lintDebug testDebugUnitTest connectedDebugAndroidTest`.

## Security & Certificates

- Generate client keypair via Android Keystore (hardware-backed ECDSA P-256).
- Export X.509 certificate (DER) for enrollment RPCs.
- Persist server certificate fingerprint post-enrollment (shared prefs/DataStore with encryption).
- Provide certificate rotation handling: prompt user if fingerprint changes.
- Implement TLS channel builder enforcing mutual TLS and hostname/IP verification.
- Expose `CertificateRepository` API covering enrollment state, revocation, and last-seen tracking.

## Enrollment Flows

### QR Code Mode

1. Onboarding screen offers Scan QR option when server QR displayed.
2. Use camera + ML Kit to parse JSON payload; validate fields and token freshness.
3. Generate client certificate, build `EnrollRequest`, invoke gRPC with TLS fingerprint pinning.
4. Handle responses: success persists `clientId`, certificate, server metadata; errors mapped to user-friendly messages.
5. Provide retry, manual entry fallback, and analytics hooks.

### Approval Mode

1. Discover servers via NSD; display list with hostname and reachability status.
2. Selecting server triggers client certificate generation and verification code computation.
3. Submit `RequestPairingRequest`; compare verification codes, display pairing UI.
4. Poll `CheckPairingStatus`; enforce timeout/cancel; surface notifications for pending status.
5. On approval, persist server fingerprint, enrollment metadata, and navigate to command dashboard.

## Command Lifecycle

1. Fetch server info (`GetServerInfo`) to surface OS, capabilities, and connection health.
2. Load commands via `ListCommands`; transform TOML metadata into UI models.
3. Render parameter forms dynamically (slider, toggle, dropdown, text) with validation.
4. Execute commands through streaming RPC; surface output log, status updates, cancellation.
5. Cache command definitions locally for offline preview; refresh on reconnect.
6. Implement tagging/filtering and favorites per PRD goals.

## UI & Navigation

- Navigation graph: `Onboarding → Enrollment → CommandDashboard → CommandDetail`.
- Compose design: Material 3, adaptive layouts for phone/tablet.
- Accessibility: content descriptions, dynamic type, high contrast.
- Theming: align with `HandControlTheme`; add icons, branding assets.
- State management: ViewModels per feature, Flow for data streams, UI tests for flows.

## Testing Strategy

- Unit: repositories, certificate utilities, verification code generation.
- Instrumentation: enrollment flows, QR scanning, NSD discovery, command execution mocks.
- Integration: gRPC channel with mock server, TLS validation tests, DataStore persistence checks.
- Security testing: certificate mismatch, expired tokens, MITM detection (verification code mismatch).
- Automation: add CI workflows for lint, detekt (optional), unit/instrumentation.

## Documentation Deliverables

- Update PRD with implementation notes as features land.
- Maintain onboarding guide for developers (environment setup, emulator configs).
- Create user-facing pairing walkthrough (QR + approval) with screenshots closer to release.
- Document troubleshooting (network discovery failures, certificate resets).

## Open Questions

- Determine server discovery behavior on restricted networks (e.g., enterprise Wi-Fi).
- Align on localization priorities and target languages.
- Decide on log export UX (share sheet? local file dump) consistent with no-telemetry stance.
