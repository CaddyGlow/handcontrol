# HandControl Project Structure

## Overview

This document defines the folder structure for the HandControl project, which consists of:
- **Server (Rust):** PC application that executes commands
- **Client (Kotlin/Android):** Mobile app that sends commands
- **Shared:** Protocol definitions (`.proto` files)

We follow modern conventions on both platforms: idiomatic async Rust with layered modules and `tracing`-based observability, and Jetpack Compose + MVVM on Android with Kotlin coroutines, Flows, and Material 3 design. Project text stays ASCII-only; emoji are not permitted in code, docs, or tooling output.

---

## Repository Layout

```
handcontrol/
├── README.md                    # Project overview and quick start
├── LICENSE                      # License file (e.g., MIT, Apache 2.0)
├── .gitignore                   # Git ignore patterns
├── Cargo.toml                   # Server package manifest (single crate, not workspace)
├── Cargo.lock                   # Dependency lock file
├── build.rs                     # Build script (proto compilation)
│
├── docs/                        # Documentation
│   ├── PRD.md                   # Product Requirements Document
│   ├── SECURITY.md              # Security architecture & fixes
│   ├── PROJECT_STRUCTURE.md     # This document
│   ├── CLAUDE.md                # Workflow guide for Claude agents
│   └── Agents.md                # Shared instructions for automated agents
│
├── proto/                       # Protocol Buffer definitions (shared)
│   ├── handcontrol.proto        # Main service definition
│   └── README.md                # Proto documentation
│
├── src/                         # Rust server source code
│   ├── main.rs                  # Entry point
│   ├── lib.rs                   # Library root (optional, for testing)
│   │
│   ├── config/                  # Configuration handling
│   │   ├── mod.rs               # Module root
│   │   ├── parser.rs            # TOML config parsing
│   │   ├── validation.rs        # Config validation
│   │   └── defaults.rs          # Default configuration
│   │
│   ├── security/                # Security & enrollment
│   │   ├── mod.rs
│   │   ├── certificates.rs      # Certificate generation/storage
│   │   ├── enrollment.rs        # Enrollment logic (QR + Approval)
│   │   ├── verification.rs      # Verification code generation
│   │   └── mtls.rs              # mTLS setup
│   │
│   ├── grpc/                    # gRPC server implementation
│   │   ├── mod.rs
│   │   ├── server.rs            # Tonic server setup
│   │   ├── enrollment.rs        # Enrollment RPCs
│   │   ├── commands.rs          # Command execution RPCs
│   │   └── middleware.rs        # Authentication middleware
│   │
│   ├── commands/                # Command execution
│   │   ├── mod.rs
│   │   ├── executor.rs          # Shell command execution
│   │   ├── parameters.rs        # Parameter substitution/validation
│   │   └── streaming.rs         # Output streaming
│   │
│   ├── mdns/                    # mDNS service discovery
│   │   ├── mod.rs
│   │   └── service.rs           # mDNS advertisement
│   │
│   ├── notifications/           # OS notification system
│   │   ├── mod.rs
│   │   ├── linux.rs             # D-Bus notifications
│   │   ├── windows.rs           # Windows Toast
│   │   ├── macos.rs             # macOS Notification Center
│   │   └── fallback.rs          # Terminal fallback
│   │
│   ├── storage/                 # Data persistence
│   │   ├── mod.rs
│   │   ├── clients.rs           # Authorized clients storage
│   │   └── paths.rs             # XDG/platform paths
│   │
│   └── utils/                   # Utilities
│       ├── mod.rs
│       ├── qr.rs                # QR code generation
│       └── logging.rs           # Logging setup
│
├── tests/                       # Integration tests
│   ├── enrollment_test.rs
│   ├── command_execution_test.rs
│   └── mtls_test.rs
│
├── examples/                    # Example configurations
│   ├── basic_config.toml
│   └── advanced_config.toml
│
├── android/                     # Android client application
│   ├── app/
│   │   ├── build.gradle.kts     # App build configuration
│   │   ├── proguard-rules.pro   # ProGuard rules
│   │   │
│   │   └── src/
│   │       ├── main/
│   │       │   ├── AndroidManifest.xml
│   │       │   ├── res/         # Android resources
│   │       │   │   ├── values/
│   │       │   │   ├── drawable/
│   │       │   │   └── layout/
│   │       │   │
│   │       │   └── kotlin/com/handcontrol/
│   │       │       │
│   │       │       ├── MainActivity.kt
│   │       │       │
│   │       │       ├── ui/      # Jetpack Compose UI
│   │       │       │   ├── screens/
│   │       │       │   │   ├── ServerListScreen.kt
│   │       │       │   │   ├── CommandListScreen.kt
│   │       │       │   │   ├── CommandExecuteScreen.kt
│   │       │       │   │   ├── QRScannerScreen.kt
│   │       │       │   │   └── PairingScreen.kt
│   │       │       │   │
│   │       │       │   ├── components/
│   │       │       │   │   ├── ServerCard.kt
│   │       │       │   │   ├── CommandCard.kt
│   │       │       │   │   ├── ParameterInput.kt
│   │       │       │   │   └── VerificationCodeDisplay.kt
│   │       │       │   │
│   │       │       │   └── theme/
│   │       │       │       ├── Theme.kt
│   │       │       │       └── Color.kt
│   │       │       │
│   │       │       ├── data/    # Data layer
│   │       │       │   ├── models/
│   │       │       │   │   ├── Server.kt
│   │       │       │   │   ├── Command.kt
│   │       │       │   │   └── PairingRequest.kt
│   │       │       │   │
│   │       │       │   ├── repository/
│   │       │       │   │   ├── ServerRepository.kt
│   │       │       │   │   └── CommandRepository.kt
│   │       │       │   │
│   │       │       │   └── storage/
│   │       │       │       ├── SecureStorage.kt  # Android Keystore
│   │       │       │       └── Preferences.kt
│   │       │       │
│   │       │       ├── network/ # gRPC client
│   │       │       │   ├── GrpcClient.kt
│   │       │       │   ├── EnrollmentService.kt
│   │       │       │   ├── CommandService.kt
│   │       │       │   └── TlsConfig.kt
│   │       │       │
│   │       │       ├── security/ # Security utilities
│   │       │       │   ├── CertificateManager.kt
│   │       │       │   ├── VerificationCode.kt
│   │       │       │   └── CertificatePinner.kt
│   │       │       │
│   │       │       ├── discovery/ # mDNS discovery
│   │       │       │   └── MdnsDiscovery.kt
│   │       │       │
│   │       │       └── viewmodel/ # ViewModels
│   │       │           ├── ServerListViewModel.kt
│   │       │           ├── CommandListViewModel.kt
│   │       │           └── PairingViewModel.kt
│   │       │
│   │       ├── androidTest/     # Instrumented tests
│   │       │   └── kotlin/
│   │       │
│   │       └── test/            # Unit tests
│   │           └── kotlin/
│   │
│   ├── gradle/                  # Gradle wrapper
│   ├── build.gradle.kts         # Project build config
│   ├── settings.gradle.kts      # Project settings
│   └── gradlew                  # Gradle wrapper script
│
├── scripts/                     # Development/deployment scripts
│   ├── generate_protos.sh       # Generate proto code for both platforms
│   ├── setup_dev.sh             # Setup development environment
│   ├── run_tests.sh             # Run all tests
│   └── create_release.sh        # Create release builds
│
└── .github/                     # GitHub workflows (if using GitHub)
    └── workflows/
        ├── rust_ci.yml          # Rust CI/CD
        └── android_ci.yml       # Android CI/CD
```

## Server Dependency Baseline

The Rust server crate tracks the following crates as of Oct 2024. Keep the versions aligned with `Cargo.toml` when bumping dependencies.

| Crate                | Version  | Role / Notes                                  |
| -------------------- | -------- | --------------------------------------------- |
| `tokio`              | 1.48.0   | Async runtime, multi-platform IO + timers     |
| `tonic`              | 0.14.2   | gRPC server framework over HTTP/2             |
| `tonic-build`        | 0.14.2   | Compile-time codegen for `.proto` files       |
| `prost`              | 0.14.1   | Protocol Buffers encoding/decoding            |
| `prost-build`        | 0.14.1   | Build-time protobuf compilation               |
| `tracing`            | 0.1.41   | Structured logging instrumentation            |
| `tracing-subscriber` | 0.3.20   | Log subscriber, filtering, fmt output         |
| `anyhow`             | 1.0.100  | Application-level error propagation           |
| `thiserror`          | 2.0.17   | Error enums for domain-specific failures      |
| `serde`              | 1.0.228  | Serialization/deserialization (configs, etc.) |
| `serde_json`         | 1.0.145  | JSON helper for telemetry payloads            |
| `toml`               | 0.9.8    | Parsing command/config TOML files             |
| `directories`        | 6.0.0    | Cross-platform config/data paths              |
| `rustls`             | 0.23.34  | TLS implementation for mTLS                   |
| `rustls-pemfile`     | 2.2.0    | PEM parsing helpers for TLS assets            |
| `webpki-roots`       | 1.0.3    | Trusted public CA roots bundle                |
| `rcgen`              | 0.14.5   | Self-signed certificate generation            |
| `mdns-sd`            | 0.15.1   | Local network service discovery               |

---

## Detailed Breakdown

### Root Level

**Key Files:**
- `Cargo.toml` - Server crate manifest (single package, non-workspace)
- `README.md` - Project overview, setup instructions
- `.gitignore` - Ignore `target/`, `build/`, `.gradle/`, etc.

### `/docs` - Documentation

All markdown documentation files:
- Technical specifications (PRD, Security)
- Development guides
- API documentation

### `/proto` - Protocol Definitions

**Purpose:** Shared protocol buffer definitions for both client and server.

**Files:**
```protobuf
// proto/handcontrol.proto
syntax = "proto3";
package handcontrol.v1;

service RemoteControl {
  rpc Enroll(...) returns (...);
  rpc RequestPairing(...) returns (...);
  // ... other RPCs
}
```

**Build Integration:**
- Rust: Compiled via `build.rs` using `tonic-build`
- Android: Compiled via Gradle using `protobuf-gradle-plugin`

### `/server` - Rust Server

**Module Organization:**

#### `config/` - Configuration Management
- Parse TOML files
- Validate command definitions
- Handle platform-specific paths (XDG on Linux)

#### `security/` - Security & Enrollment
- Certificate generation (using `rcgen`)
- Enrollment token management
- Verification code generation
- mTLS configuration (using `rustls`)

#### `grpc/` - gRPC Server
- Tonic server setup
- RPC implementations
- Authentication middleware

#### `commands/` - Command Execution
- Spawn shell processes
- Parameter substitution with escaping
- Stream stdout/stderr back to client

#### `mdns/` - Service Discovery
- Broadcast `_handcontrol._tcp.local.`
- Include TXT records (version, cert fingerprint)

#### `notifications/` - OS Notifications
- Platform-specific implementations
- Used for approval mode pairing requests

#### `storage/` - Data Persistence
- Store authorized client certificates
- Manage `metadata.toml`
- Handle XDG paths

**Root Cargo.toml (Single Package):**
```toml
[package]
name = "handcontrol-server"
version = "0.1.0"
edition = "2024"
authors = ["Your Name <you@example.com>"]
description = "Remote command execution server for HandControl"
license = "MIT OR Apache-2.0"

[[bin]]
name = "handcontrol-server"
path = "src/main.rs"

[dependencies]
tokio = { version = "1", features = ["full"] }
tonic = "0.12"
prost = "0.13"
toml = "0.8"
serde = { version = "1", features = ["derive"] }
directories = "5"
rcgen = "0.13"
rustls = "0.23"
mdns-sd = "0.11"
uuid = { version = "1", features = ["v4", "serde"] }
tracing = "0.1"
tracing-subscriber = "0.3"

[target.'cfg(target_os = "linux")'.dependencies]
notify-rust = "4"

[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "0.58", features = ["UI.Notifications"] }

[target.'cfg(target_os = "macos")'.dependencies]
mac-notification-sys = "0.6"

[build-dependencies]
tonic-build = "0.12"

[dev-dependencies]
tempfile = "3"
```

**Root build.rs:**
```rust
fn main() {
    tonic_build::configure()
        .build_server(true)
        .build_client(false)
        .out_dir("src/grpc/generated")
        .compile(&["proto/handcontrol.proto"], &["proto"])
        .unwrap();
}
```

### `/android` - Android Client

**Architecture:** MVVM with Jetpack Compose

#### `ui/` - User Interface
- **Compose screens:** Each screen is a composable function
- **Components:** Reusable UI elements
- **Theme:** Material 3 theming

#### `data/` - Data Layer
- **Models:** Data classes for Server, Command, etc.
- **Repository:** Data access abstraction
- **Storage:** Secure storage using Android Keystore

#### `network/` - gRPC Client
- Kotlin coroutines for async operations
- gRPC-Kotlin client
- TLS configuration with certificate pinning

#### `security/` - Security Utilities
- Certificate generation (using BouncyCastle or Android APIs)
- Verification code generation
- Certificate pinning logic

#### `discovery/` - mDNS Discovery
- Android NSD (Network Service Discovery) API
- Find `_handcontrol._tcp.local.` services

#### `viewmodel/` - Business Logic
- AndroidX ViewModel
- State management
- Handle user interactions

**build.gradle.kts Example:**
```kotlin
plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("com.google.protobuf")
}

android {
    namespace = "com.handcontrol"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.handcontrol"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "1.0"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
    }

    composeOptions {
        kotlinCompilerExtensionVersion = "1.5.8"
    }
}

dependencies {
    // Compose
    implementation("androidx.compose.ui:ui:1.6.8")
    implementation("androidx.compose.material3:material3:1.2.1")
    implementation("androidx.compose.ui:ui-tooling-preview:1.6.8")
    implementation("androidx.activity:activity-compose:1.9.0")

    // gRPC
    implementation("io.grpc:grpc-kotlin-stub:1.4.1")
    implementation("io.grpc:grpc-okhttp:1.65.1")
    implementation("com.google.protobuf:protobuf-kotlin:4.27.1")

    // Coroutines
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")

    // ViewModel
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.2")

    // Security
    implementation("androidx.security:security-crypto:1.1.0-alpha06")

    // Camera (for QR scanning)
    implementation("androidx.camera:camera-camera2:1.3.4")
    implementation("com.google.mlkit:barcode-scanning:17.2.0")
}

protobuf {
    protoc {
        artifact = "com.google.protobuf:protoc:4.27.1"
    }
    plugins {
        id("grpc") {
            artifact = "io.grpc:protoc-gen-grpc-java:1.65.1"
        }
        id("grpckt") {
            artifact = "io.grpc:protoc-gen-grpc-kotlin:1.4.1:jdk8@jar"
        }
    }
    generateProtoTasks {
        all().forEach {
            it.plugins {
                id("grpc")
                id("grpckt")
            }
            it.builtins {
                id("kotlin")
            }
        }
    }
}
```

### `/scripts` - Development Scripts

**generate_protos.sh:**
```bash
#!/bin/bash
# Generate protocol buffer code for both platforms

echo "Generating Rust proto code..."
cargo build

echo "Generating Kotlin proto code..."
cd android && ./gradlew generateProto
cd ..

echo "Proto generation complete!"
```

**setup_dev.sh:**
```bash
#!/bin/bash
# Setup development environment

echo "Installing Rust toolchain..."
rustup update stable

echo "Installing Android Studio dependencies..."
# Instructions for setting up Android SDK

echo "Installing protocol buffer compiler..."
# Platform-specific protoc installation

echo "Setup complete!"
```

---

## Configuration File Locations

### Server (Linux)
```
~/.config/handcontrol/
├── config.toml              # User configuration
├── server.crt               # Server certificate
├── server.key               # Server private key
└── authorized_clients/
    ├── metadata.toml        # Client registry
    ├── client1.crt
    └── client2.crt
```

### Server (Windows)
```
%APPDATA%\handcontrol\
├── config.toml
├── server.crt
├── server.key
└── authorized_clients\
    └── ...
```

### Server (macOS)
```
~/Library/Application Support/handcontrol/
├── config.toml
├── server.crt
├── server.key
└── authorized_clients/
    └── ...
```

### Android
```
/data/data/com.handcontrol/
├── shared_prefs/            # Preferences (server list, etc.)
├── files/                   # App data
└── (Android Keystore)       # Certificates (hardware-backed)
```

---

## Build Artifacts

### Ignored by Git (`.gitignore`)
```gitignore
# Rust
target/
Cargo.lock

# Android
android/.gradle/
android/app/build/
android/local.properties
*.apk
*.aab

# Generated proto code
server/src/grpc/generated/
android/app/build/generated/

# IDE
.idea/
.vscode/
*.swp

# OS
.DS_Store
Thumbs.db

# Secrets (important!)
*.key
*.crt
config.toml
authorized_clients/
```

---

## Future: Migrating to Workspace (Optional)

If you later add more Rust crates (CLI tool, shared library, etc.), you can easily migrate to a workspace:

**Step 1:** Move server code to subdirectory:
```bash
mkdir server
mv src Cargo.toml build.rs server/
```

**Step 2:** Create workspace root `Cargo.toml`:
```toml
[workspace]
members = ["server"]
resolver = "2"

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
tonic = "0.12"
prost = "0.13"
serde = { version = "1", features = ["derive"] }
```

**Step 3:** Update `server/Cargo.toml` to use workspace dependencies:
```toml
[package]
name = "handcontrol-server"
# ...

[dependencies]
tokio = { workspace = true, features = ["full"] }
tonic = { workspace = true }
# ...
```

**But for now:** Keep it simple with a single package!

---

## Development Workflow

### Initial Setup
```bash
# Clone repository
git clone <repo-url>
cd handcontrol

# Setup environment
./scripts/setup_dev.sh

# Generate proto code
./scripts/generate_protos.sh
```

### Running Server (Development)
```bash
cargo run
```

### Running Android (Development)
```bash
cd android
./gradlew installDebug
adb shell am start -n com.handcontrol/.MainActivity
```

### Running Tests
```bash
# Rust tests
cargo test

# Android tests
cd android
./gradlew test
./gradlew connectedAndroidTest
```

---

## Release Structure

### Server Releases
```
handcontrol-server-v1.0.0-linux-x86_64.tar.gz
├── handcontrol-server         # Binary
├── config.example.toml        # Example config
└── README.md                  # Installation instructions

handcontrol-server-v1.0.0-windows-x86_64.zip
├── handcontrol-server.exe
├── config.example.toml
└── README.txt

handcontrol-server-v1.0.0-macos-universal.tar.gz
├── handcontrol-server
├── config.example.toml
└── README.md
```

### Android Releases
```
handcontrol-v1.0.0.apk         # Direct APK
handcontrol-v1.0.0.aab         # Play Store bundle
```

---

## Workspace Decision Summary

### ✅ Recommended: Single Package (No Workspace)

**Current structure:**
```
handcontrol/
├── Cargo.toml         # Single package
├── build.rs
├── src/               # Server code directly here
├── proto/
└── android/
```

**Reasons:**
- ✅ Simpler for single Rust binary
- ✅ Faster compile times
- ✅ Less configuration overhead
- ✅ Easier to understand for contributors
- ✅ Standard Rust project layout

**Commands:**
```bash
cargo build           # Build server
cargo run             # Run server
cargo test            # Test server
```

### When to Migrate to Workspace

Only migrate when you actually need multiple Rust crates:
- Adding a CLI management tool
- Extracting shared library
- Creating separate test utilities
- Building platform-specific variants

Migration is straightforward and can be done anytime. **Start simple!**

---

## Alternative Structures (Considerations)

### Monorepo vs Multi-repo

**Current (Monorepo):**
```
handcontrol/
├── server/
└── android/
```

**Pros:**
- Shared proto definitions easy to keep in sync
- Single version tag for releases
- Easier CI/CD coordination

**Cons:**
- Larger repository
- Different language tooling in one place

**Alternative (Multi-repo):**
```
handcontrol-server/    (separate repo)
handcontrol-android/   (separate repo)
handcontrol-proto/     (shared as git submodule)
```

**Recommendation:** Start with monorepo for simplicity. Can split later if needed.

---

## Summary

This structure provides:
- ✅ Clear separation of concerns
- ✅ Platform-specific organization
- ✅ Shared protocol definitions
- ✅ Standard Rust/Android conventions
- ✅ Easy to navigate and extend
- ✅ CI/CD friendly
- ✅ Proper security (secrets not in repo)

Ready to start implementation?
