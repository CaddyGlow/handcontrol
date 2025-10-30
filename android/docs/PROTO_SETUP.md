# Protocol Buffer Setup

## Overview

The Android client uses the shared proto file from the parent project located at `../proto/handcontrol.proto`. This ensures that the client and server stay in sync and use the same message definitions.

## Configuration

The Android build is configured to use the shared proto file via the `build.gradle.kts` sourceSets configuration:

```kotlin
android {
    // ...
    sourceSets {
        getByName("main") {
            proto {
                // Use shared proto file from parent project
                srcDir("${project.rootDir}/../proto")
            }
        }
    }
}
```

## Proto File Location

- **Shared Proto**: `../proto/handcontrol.proto` (parent project)
- **Old Android Proto**: `app/src/main/proto/handcontrol.proto.backup` (backed up, no longer used)

## Java-Specific Options

The shared proto file includes Java-specific options for Android code generation:

```protobuf
option java_multiple_files = true;
option java_package = "com.handcontrol.grpc";
option java_outer_classname = "HandControlProto";
```

These options ensure proper Java/Kotlin code generation for the Android client.

## Generated Code

Generated proto code is created during the build process and placed in:

- Java stubs: `app/build/generated/source/proto/debug/java/com/handcontrol/grpc/`
- gRPC stubs: `app/build/generated/source/proto/debug/grpc/com/handcontrol/grpc/`
- Kotlin stubs: `app/build/generated/source/proto/debug/kotlin/`
- gRPC Kotlin stubs: `app/build/generated/source/proto/debug/grpckt/`

## Regenerating Proto Code

To regenerate proto code after changes to the shared proto file:

```bash
cd android
./gradlew clean :app:generateDebugProto
```

Or to build the entire app (which includes proto generation):

```bash
./gradlew :app:assembleDebug
```

## Available RPCs

The shared proto file defines the following RPCs (some are server-side only):

### Client-Side RPCs (Used by Android)
- `Enroll` - QR code enrollment
- `RequestPairing` - Approval mode pairing request
- `CheckPairingStatus` - Poll pairing status
- `GetServerInfo` - Get server metadata
- `ListCommands` - Fetch available commands
- `ExecuteCommand` - Execute command with streaming output

### Server-Side RPCs (Not used by Android)
- `GenerateEnrollmentQR` - Generate QR code on server (CLI use)
- `ApprovePairing` - Programmatically approve pairing (CLI use)
- `ListPendingPairings` - List pending pairings (CLI use)
- `GetConfigVersion` - Config hot-reload API
- `WatchConfigUpdates` - Config change notifications

## Proto Lite

The Android client uses proto-lite for smaller APK size:

```kotlin
protobuf {
    generateProtoTasks {
        all().forEach { task ->
            task.plugins {
                id("grpc") { option("lite") }
                id("grpckt") { option("lite") }
            }
            task.builtins {
                id("kotlin") { option("lite") }
                id("java") { option("lite") }
            }
        }
    }
}
```

## Keeping Proto Files in Sync

When making changes to the proto file:

1. Edit `../proto/handcontrol.proto` (shared proto)
2. Regenerate Rust code: `cd .. && cargo build`
3. Regenerate Android code: `cd android && ./gradlew :app:generateDebugProto`
4. Update server implementation if needed
5. Update Android implementation if needed

## Version Control

- The shared proto file at `../proto/handcontrol.proto` is tracked in git
- Generated proto code in `app/build/` is NOT tracked (in .gitignore)
- The old Android-specific proto is backed up at `app/src/main/proto/handcontrol.proto.backup`
