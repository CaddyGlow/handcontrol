# HandControl Android Client

Android client application for HandControl remote command execution system.

## Architecture

- **Language**: Kotlin
- **UI Framework**: Jetpack Compose
- **Architecture Pattern**: MVVM
- **Design System**: Material 3
- **Dependency Injection**: Hilt
- **gRPC Client**: grpc-kotlin + protobuf-kotlin-lite

## Project Structure

```
app/src/main/kotlin/com/handcontrol/
├── MainActivity.kt
├── HandControlApplication.kt
├── core/
│   ├── discovery/           # mDNS service discovery
│   ├── network/             # gRPC channel setup
│   ├── security/            # Certificate management, verification
│   └── logging/             # Logging initialization
├── data/
│   ├── commands/            # Command repository
│   ├── enrollment/          # Enrollment repository
│   └── database/            # Room database for enrolled servers
├── di/                      # Dependency injection modules
├── feature/
│   ├── welcome/             # Welcome screen
│   ├── discovery/           # Server discovery screen
│   ├── enrollment/          # QR & approval pairing
│   ├── serverlist/          # Enrolled servers list
│   └── commands/            # Command list & execution
├── navigation/              # Navigation graph
└── ui/
    └── theme/               # Material 3 theme
```

## Protocol Buffers

The Android app uses the **shared proto file** from the parent project:

- **Proto Location**: `../proto/handcontrol.proto`
- **Configuration**: See `android/docs/PROTO_SETUP.md`

To regenerate proto code:
```bash
./gradlew :app:generateDebugProto
```

## Building

### Requirements

- JDK 17+
- Android SDK (API 35)
- Gradle 8.14+

### Build Commands

```bash
# Clean build
./gradlew clean

# Build debug APK
./gradlew :app:assembleDebug

# Build and install on connected device
./gradlew :app:installDebug

# Run tests
./gradlew :app:test
./gradlew :app:connectedAndroidTest
```

### Quick Build & Install Script

```bash
./build-install-run.sh
```

## Features

### Enrollment

- **QR Code Mode**: Scan QR code from PC for instant pairing
- **Approval Mode**: Interactive pairing with verification code

### Certificate Management

- Client certificates stored in Android Keystore (hardware-backed)
- Server certificate pinning (TOFU - Trust On First Use)
- ECDSA P-256 certificates for optimal mobile performance

### Security

- mTLS (mutual TLS) for all authenticated connections
- Verification code prevents MITM attacks during pairing
- Nonce-based verification prevents precomputation attacks

### Command Execution

- Real-time streaming command output
- Support for command parameters:
  - Sliders (numeric range)
  - Text inputs (with regex validation)
  - Toggles (boolean)
  - Dropdowns (selection)

### Server Discovery

- mDNS/NSD for automatic server discovery
- Manual server entry support
- Persistent enrolled servers in Room database

## Configuration

### Minimum SDK

- **minSdk**: 24 (Android 7.0)
- **targetSdk**: 35 (Android 15)

### Dependencies

See `app/build.gradle.kts` for full dependency list.

Key dependencies:
- Jetpack Compose BOM: 2025.10.01
- gRPC: 1.66.0
- grpc-kotlin-stub: 1.4.1
- Hilt: 2.52
- Room: 2.6.1

## Development

### Code Style

- Follow Kotlin coding conventions
- Use Compose best practices
- Maintain MVVM separation of concerns

### Testing

Unit tests:
```bash
./gradlew :app:test
```

Instrumented tests:
```bash
./gradlew :app:connectedAndroidTest
```

## Proto File Sync

The Android app shares the same proto file with the Rust server to ensure message compatibility.

**Important**: After modifying `../proto/handcontrol.proto`:

1. Regenerate Rust code: `cd .. && cargo build`
2. Regenerate Android code: `./gradlew :app:generateDebugProto`
3. Update implementations as needed

See `docs/PROTO_SETUP.md` for detailed information.

## Related Documentation

- [Protocol Buffer Setup](docs/PROTO_SETUP.md)
- [Main Project PRD](../docs/PRD.md)
- [Security Architecture](../docs/SECURITY.md)
- [Project Structure](../docs/PROJECT_STRUCTURE.md)
