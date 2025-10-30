# Product Requirements Document: HandControl

**Version:** 1.0
**Date:** 2025-10-29
**Status:** Draft

## Overview

HandControl is a secure remote control system that allows Android devices to execute predefined commands on a PC over a local network. The system consists of a PC server application (Rust) and an Android mobile client (Kotlin).

## Goals

- Provide secure, easy-to-use remote command execution
- Simple pairing via QR code or interactive approval
- Flexible command configuration via human-editable TOML files
- Support multiple mobile clients per PC
- Cross-platform PC support (Linux, Windows, macOS)

## Non-Goals

- File transfer capabilities
- Bi-directional control (PC to Android)
- VPN/different subnet support
- Command chaining/macros (not in v1)
- Remote config updates
- Command approval workflows

---

## Architecture

### System Components

```
┌─────────────────────────────────────────┐
│       Android Client (Kotlin)           │
│  ┌────────────────────────────────────┐ │
│  │   Jetpack Compose UI               │ │
│  └──────────────┬─────────────────────┘ │
│                 │                        │
│  ┌──────────────▼─────────────────────┐ │
│  │  gRPC Kotlin Client                │ │
│  │  - mTLS authentication             │ │
│  │  - Certificate storage (Keystore)  │ │
│  └──────────────┬─────────────────────┘ │
└─────────────────┼───────────────────────┘
                  │
                  │ gRPC over HTTP/2
                  │ mTLS encrypted
                  │
┌─────────────────▼───────────────────────┐
│      PC Server (Rust/Tokio/Tonic)       │
│  ┌────────────────────────────────────┐ │
│  │     Tonic gRPC Server              │ │
│  │     - mTLS authentication          │ │
│  │     - mDNS service registration    │ │
│  │     - Enrollment token validation  │ │
│  └──────────────┬─────────────────────┘ │
│                 │                        │
│  ┌──────────────▼─────────────────────┐ │
│  │   Command Executor                 │ │
│  │   - TOML config parsing            │ │
│  │   - Shell process spawning         │ │
│  │   - Output streaming               │ │
│  └────────────────────────────────────┘ │
└──────────────────────────────────────────┘
```

### Technology Stack

**PC Server:**
- Language: Rust
- Runtime: Tokio (async)
- gRPC: Tonic + Prost
- Config: TOML (via `toml` crate)
- Paths: `directories` crate (XDG on Linux, platform equivalents)
- Certificates: `rcgen` (generation), `rustls` (TLS)
- mDNS: `mdns-sd` or similar
- Dependency policy: Prefer pure-Rust crates with minimal native bindings to keep cross-compilation (Linux/Windows/macOS/Android) straightforward

**Server Crate Versions (Oct 2024)**

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

**Android Client:**
- Language: Kotlin
- UI: Jetpack Compose
- gRPC: `grpc-kotlin` + `protobuf-kotlin`
- Certificate Storage: Android Keystore
- mDNS: NSD (Network Service Discovery) API

**Shared:**
- Protocol definitions: Protocol Buffers (`.proto` files)

---

## Security Model

### Authentication Flow

HandControl supports **two enrollment modes** that can be configured on the server:

1. **QR Code Mode** (token-based, user scans QR)
2. **Approval Mode** (interactive, user approves on PC)

The mode is configurable via the server config file and can be disabled/enabled independently.

---

#### Enrollment Mode 1: QR Code Pairing

**When to use:** User has physical access to PC, wants quick pairing without interaction.

1. **Server generates enrollment session:**
   ```
   - Self-signed certificate (if not exists)
   - One-time enrollment token (UUID)
   - Token expires after first use or 5 minutes
   ```

2. **QR code contains:**
   ```json
   {
     "ip": "192.168.1.100",
     "port": 50051,
     "cert_fingerprint": "SHA256:abc123...",
     "enrollment_token": "550e8400-e29b-41d4-a716-446655440000",
     "server_id": "uuid-of-server"
   }
   ```

3. **Android scans QR:**
   - Validates JSON structure
   - Generates client certificate (self-signed X.509, ECDSA P-256, 10-year validity)
   - Private key stored in Android Keystore hardware-backed (never transmitted)
   - Connects to server via TLS
   - Verifies server certificate fingerprint matches QR code

4. **Enrollment RPC:**
   ```protobuf
   message EnrollRequest {
     string enrollment_token = 1;
     bytes client_certificate = 2;  // Full X.509 certificate (DER-encoded)
     string device_name = 3;         // "My Pixel 7"
   }

   message EnrollResponse {
     bool success = 1;
     string client_id = 2;  // UUID assigned by server
     string error_message = 3;
   }
   ```

   **Certificate Format:**
   - Android generates self-signed X.509 certificate with ECDSA P-256, 10-year validity
   - Private key stored in Android Keystore hardware-backed (never transmitted)
   - Full certificate (including ECDSA P-256 public key) sent to server
   - Server stores complete certificate for mTLS validation

5. **Server validates:**
   - Token exists and not expired
   - Stores full client certificate in `~/.config/handcontrol/authorized_clients/`
   - Invalidates token
   - Returns success

---

#### Enrollment Mode 2: Approval-Based Pairing with Verification Code

**When to use:**
- User prefers not to use camera/QR scanning
- Adding multiple devices (easier than generating QR each time)
- User has access to PC notifications (physically present or via remote desktop/SSH + notification forwarding)
- More user-friendly for non-technical users

**Prerequisites:**
- Server must have notification system access (Linux: D-Bus notifications, Windows: Toast notifications, macOS: Notification Center)
- User must be able to see PC notifications (either physically present or via remote session)
- Approval mode enabled in config

**Note:** While this mode doesn't require physical access to the PC's main display, the user MUST be able to see the verification code shown in the PC's notification. This can be:
- Notification on PC screen (user physically present)
- Notification forwarded to mobile via remote desktop apps
- Notification visible in SSH session with notification forwarding
- **This mode is NOT suitable for truly headless/unattended pairing**

**Security Model:**
- Uses **TOFU (Trust On First Use)** with verification code
- Verification code prevents man-in-the-middle attacks
- User must verify code matches on both devices (like Bluetooth pairing)

**Flow:**

1. **Android discovers server via mDNS:**
   - Finds server broadcasting `_handcontrol._tcp.local.`
   - Displays server name (hostname)
   - Gets server's certificate fingerprint from mDNS TXT record
   - User taps "Request Pairing"

2. **Android initiates pairing request (over TLS, server cert not yet pinned):**
   - Generates client certificate (self-signed X.509, ECDSA P-256)
   - Computes verification code from both certificates

   ```protobuf
   message RequestPairingRequest {
     string device_name = 1;          // "My Pixel 7"
     string device_model = 2;         // "Google Pixel 7" (optional)
     bytes client_certificate = 3;    // Full X.509 certificate (DER-encoded)
     string verification_code = 4;    // 6-digit code derived from cert exchange
   }

   message RequestPairingResponse {
     bool pending = 1;
     string pairing_request_id = 2;
     int32 timeout_seconds = 3;
     string verification_code = 4;    // Server's computed verification code
     bytes server_cert_fingerprint = 5; // SHA256 of server cert
     string error_message = 6;        // If verification code mismatch or other error
   }
   ```

   **Certificate Format:**
   - Android generates self-signed X.509 certificate with ECDSA P-256, 10-year validity
   - Private key stored in Android Keystore hardware-backed (never transmitted)
   - Full certificate (including ECDSA P-256 public key) sent to server
   - Server stores complete certificate for mTLS validation

3. **Verification code generation:**
   ```
   # Both client and server compute the same code independently
   # Using full certificate fingerprints ensures authenticity
   client_fingerprint = SHA256(client_certificate_DER)
   server_fingerprint = SHA256(server_certificate_DER)
   code_material = SHA256(client_fingerprint || server_fingerprint || server_id)
   verification_code = first_6_digits(code_material)
   # Results in: "482-917" (formatted as XXX-XXX for readability)
   ```

4. **Both sides display verification code:**

   **Android shows:**
   ```
   ┌──────────────────────────────┐
   │  Pairing Request Pending     │
   ├──────────────────────────────┤
   │  Server: Work Laptop         │
   │                              │
   │  Verification Code:          │
   │  ┏━━━━━━━━━━━━━━━━━━━━━━━┓  │
   │  ┃      482-917          ┃  │
   │  ┗━━━━━━━━━━━━━━━━━━━━━━━┛  │
   │                              │
   │  Verify this code matches    │
   │  on your computer            │
   │                              │
   │  Timeout: 60s                │
   │        [Cancel]              │
   └──────────────────────────────┘
   ```

   **Server shows notification:**
   ```
   Title: HandControl Pairing Request
   Body: Device "My Pixel 7" wants to connect
         Verification Code: 482-917
   Actions: [Accept] [Reject]
   ```

5. **Server validates verification code (MANDATORY):**
   - Server receives `RequestPairingRequest` with client's verification code
   - Server computes its own verification code using same algorithm
   - **If codes match:** Proceed to show notification (no MITM)
   - **If codes DON'T match:** Return error immediately, do NOT show notification
     ```protobuf
     RequestPairingResponse {
       pending = false
       error_message = "Verification failed - possible MITM attack"
     }
     ```
   - This check MUST happen before any user interaction
   - Prevents MITM from enrolling by swapping certificates

6. **Android validates server response:**
   - Confirms `pending = true` in `RequestPairingResponse`
   - Recomputes expected verification code and verifies it matches the server-provided value
   - Validates `server_cert_fingerprint` equals the SHA256 fingerprint of the TLS certificate received on this connection
   - On any mismatch, aborts pairing and shows a "Security verification failed" error instead of displaying the code
   - Only after these checks pass does Android present the verification code UI

7. **User verification (critical security step):**
   - User checks that code on phone matches code in PC notification
   - **If codes match:** User clicks [Accept] on PC (codes derived from same certs = no MITM)
   - **If codes DON'T match:** MITM attack detected! User clicks [Reject]

8. **Android polls for approval:**
   ```protobuf
   message CheckPairingStatusRequest {
     string pairing_request_id = 1;
   }

   message CheckPairingStatusResponse {
     PairingStatus status = 1;
     string client_id = 2;          // UUID if approved
     string error_message = 3;
   }

   enum PairingStatus {
     PAIRING_STATUS_UNSPECIFIED = 0;
     PAIRING_STATUS_PENDING = 1;
     PAIRING_STATUS_APPROVED = 2;
     PAIRING_STATUS_REJECTED = 3;
     PAIRING_STATUS_TIMEOUT = 4;
   }
   ```
   - Android polls every 2 seconds (max 60 seconds)

9. **User responds to notification:**
   - **Accept:** Server stores client certificate, returns `APPROVED`
   - **Reject:** Server returns `REJECTED`, removes pending request
   - **Timeout:** After 60 seconds, returns `TIMEOUT`

10. **Android receives approval:**
   - Status = `APPROVED`:
     - Pins server certificate (reject future changes)
     - Saves client cert and client ID
     - Shows "Successfully paired"
   - Status = `REJECTED` or `TIMEOUT`: Shows error

**Security Properties:**
- **Prevents MITM:** Verification code is derived from actual certificates exchanged
- **Out-of-band verification:** User visually compares codes (like Bluetooth)
- **No pre-shared secrets needed:** Uses public key cryptography
- **Replay protection:** Pairing request ID is single-use

---

#### Configuration

```toml
[security.enrollment]
# Enable/disable enrollment modes
qr_code_enabled = true           # QR code token-based pairing
approval_enabled = true          # Interactive approval pairing

# Approval mode settings
approval_timeout_seconds = 60    # How long to wait for user response
approval_notification = true     # Show OS notifications (requires system integration)
```

**Behavior:**
- Both modes can be enabled simultaneously (user chooses on Android)
- If only QR enabled: Android only shows "Scan QR" option
- If only approval enabled: Android only shows "Connect" button after mDNS discovery
- If both enabled: Android shows both options

#### Phase 2: Authenticated Connections (mTLS)

All subsequent connections use mutual TLS:
- Server validates client certificate against authorized list
- Client validates server certificate fingerprint
- Connection rejected if either validation fails

### Certificate Management

**Server Certificates:**
- **Location:** `~/.config/handcontrol/server.crt` and `server.key`
- **Generation:** Auto-generated on first run using `rcgen`
- **Algorithm:** ECDSA P-256 (secp256r1) for smaller key sizes and better performance
- **Validity:** 10 years (long-lived)
- **Rotation:** Manual (user deletes files, server regenerates)

**Client Certificates:**
- **Storage:** Android Keystore (hardware-backed)
- **Generation:** On-device during enrollment
- **Algorithm:** ECDSA P-256 (secp256r1) - hardware-backed on most modern devices
- **Validity:** 10 years
- **Revocation:** Server removes from `authorized_clients/` directory

**Authorized Clients Registry:**
```toml
# ~/.config/handcontrol/authorized_clients/metadata.toml

[[client]]
id = "550e8400-e29b-41d4-a716-446655440000"
name = "My Pixel 7"
cert_fingerprint = "SHA256:xyz789..."
enrolled_at = "2025-10-29T10:30:00Z"
last_seen = "2025-10-29T14:22:00Z"

[[client]]
id = "another-uuid"
name = "Galaxy Tab"
cert_fingerprint = "SHA256:abc123..."
enrolled_at = "2025-10-28T09:15:00Z"
last_seen = "2025-10-29T12:00:00Z"
```

---

## Network Discovery

### mDNS Service Advertisement

**Server broadcasts:**
- Service type: `_handcontrol._tcp.local.`
- Instance name: Defaults to system hostname (configurable)
- Port: From config (default 50051)
- TXT records:
  - `version=1.0`
  - `server_id=<uuid>`
  - `cert_fingerprint=<SHA256>`

**Android discovers:**
- Uses Android NSD API to find `_handcontrol._tcp.local.` services
- Displays list of discovered servers
- Shows instance name (hostname)
- User selects server to pair with

### Manual Connection

User can manually enter:
- IP address
- Port (defaults to 50051)

---

## Configuration

### Server Config File

**Location:**
- Linux: `~/.config/handcontrol/config.toml`
- Windows: `%APPDATA%\handcontrol\config.toml`
- macOS: `~/Library/Application Support/handcontrol/config.toml`

**Schema:**

```toml
[server]
# Network settings
port = 50051
# Bind address (0.0.0.0 for all interfaces, 127.0.0.1 for localhost only)
bind_address = "0.0.0.0"

# mDNS settings
mdns_service_name = "handcontrol"
# Instance name shown to clients (defaults to system hostname if not set)
mdns_instance_name = "My Desktop PC"

[security]
# Certificate paths (auto-generated if not exist)
cert_path = "~/.config/handcontrol/server.crt"
key_path = "~/.config/handcontrol/server.key"
authorized_clients_dir = "~/.config/handcontrol/authorized_clients"

# Enrollment token expiry (seconds)
enrollment_token_ttl = 300

# Command definitions
[[command]]
id = "lock-screen"                           # Unique identifier
name = "Lock Screen"                         # Display name
description = "Locks the computer screen"    # Description shown in UI
icon = "lock"                                # Icon identifier (optional)
shell = "loginctl lock-session"              # Shell command to execute
tags = ["system", "security"]                # Tags for filtering/organization
timeout_seconds = 5                          # Command execution timeout

# Optional: Environment variables for this command
[command.env]
DISPLAY = ":0"

[[command]]
id = "set-volume"
name = "Set Volume"
description = "Adjust system volume"
shell = "pactl set-sink-volume @DEFAULT_SINK@ {level}%"
tags = ["media", "audio"]
timeout_seconds = 3

# Parameters define UI controls on Android
[[command.parameters]]
name = "level"                               # Parameter name (used in {level})
type = "slider"                              # UI control type
min = 0                                      # Slider minimum
max = 100                                    # Slider maximum
default = 50                                 # Default value
description = "Volume level (0-100)"        # Help text

[[command]]
id = "toggle-mute"
name = "Toggle Mute"
shell = "pactl set-sink-mute @DEFAULT_SINK@ {muted}"
tags = ["media", "audio"]

[[command.parameters]]
name = "muted"
type = "toggle"                              # Boolean toggle
default = true
label_on = "Mute"                            # Label when true
label_off = "Unmute"                         # Label when false

[[command]]
id = "switch-audio-output"
name = "Switch Audio Output"
shell = "pactl set-default-sink {output}"
tags = ["media", "audio"]

[[command.parameters]]
name = "output"
type = "dropdown"                            # Dropdown selection
options = ["speakers", "headphones", "hdmi"]
default = "speakers"
description = "Select audio output device"

[[command]]
id = "custom-script"
name = "Run Script"
shell = "/home/user/scripts/{script_name}.sh {arg}"
tags = ["custom"]

[[command.parameters]]
name = "script_name"
type = "text"                                # Text input
description = "Script filename (without .sh)"
validation = "^[a-zA-Z0-9_-]+$"             # Regex validation (optional)

[[command.parameters]]
name = "arg"
type = "text"
description = "Script argument"
default = ""
```

### Parameter Types

| Type | Description | Required Fields | Optional Fields |
|------|-------------|----------------|-----------------|
| `slider` | Numeric slider | `min`, `max` | `default`, `step`, `description` |
| `text` | Text input | - | `default`, `validation` (regex), `description` |
| `toggle` | Boolean switch | - | `default`, `label_on`, `label_off`, `description` |
| `dropdown` | Selection list | `options` (array) | `default`, `description` |

### Parameter Substitution

- Parameters are substituted in shell commands using `{parameter_name}` syntax
- Values are shell-escaped to prevent injection
- Example: `"amixer set Master {volume}%"` with `volume=50` becomes `"amixer set Master 50%"`

---

## Protocol Definition

### gRPC Service

```protobuf
syntax = "proto3";

package handcontrol.v1;

// Main service
service RemoteControl {
  // Enrollment: QR code mode (token-based)
  rpc Enroll(EnrollRequest) returns (EnrollResponse);

  // Enrollment: Approval mode (interactive)
  rpc RequestPairing(RequestPairingRequest) returns (RequestPairingResponse);
  rpc CheckPairingStatus(CheckPairingStatusRequest) returns (CheckPairingStatusResponse);
  rpc ApprovePairing(ApprovePairingRequest) returns (ApprovePairingResponse);
  rpc ListPendingPairings(ListPendingPairingsRequest) returns (ListPendingPairingsResponse);

  // Get server information
  rpc GetServerInfo(ServerInfoRequest) returns (ServerInfoResponse);

  // List available commands
  rpc ListCommands(ListCommandsRequest) returns (ListCommandsResponse);

  // Execute a command with streaming output
  rpc ExecuteCommand(ExecuteCommandRequest) returns (stream ExecuteCommandResponse);
}

// Enrollment (QR code mode)
message EnrollRequest {
  string enrollment_token = 1;
  bytes client_certificate = 2;  // Full X.509 certificate (DER-encoded)
  string device_name = 3;
}

message EnrollResponse {
  bool success = 1;
  string client_id = 2;
  string error_message = 3;
}

// Enrollment (Approval mode with verification code)
message RequestPairingRequest {
  string device_name = 1;
  string device_model = 2;
  bytes client_certificate = 3;     // Full X.509 certificate (DER-encoded)
  string verification_code = 4;     // Client's computed verification code
}

message RequestPairingResponse {
  bool pending = 1;
  string pairing_request_id = 2;
  int32 timeout_seconds = 3;
  string verification_code = 4;     // Server's computed verification code
  bytes server_cert_fingerprint = 5; // SHA256 of server certificate
  string error_message = 6;         // If verification mismatch, approval mode disabled, or other error
}

message CheckPairingStatusRequest {
  string pairing_request_id = 1;
}

message CheckPairingStatusResponse {
  PairingStatus status = 1;
  string client_id = 2;
  string error_message = 3;
}

enum PairingStatus {
  PAIRING_STATUS_UNSPECIFIED = 0;
  PAIRING_STATUS_PENDING = 1;
  PAIRING_STATUS_APPROVED = 2;
  PAIRING_STATUS_REJECTED = 3;
  PAIRING_STATUS_TIMEOUT = 4;
}

// Programmatic pairing approval (CLI/automation)
message ApprovePairingRequest {
  string pairing_request_id = 1;
}

message ApprovePairingResponse {
  bool success = 1;
  string client_id = 2;
  string error_message = 3;
}

// List pending pairing requests
message ListPendingPairingsRequest {}

message ListPendingPairingsResponse {
  repeated PendingPairingInfo pending_pairings = 1;
}

message PendingPairingInfo {
  string request_id = 1;
  string device_name = 2;
  string device_model = 3;
  string verification_code = 4;
  int64 expires_at_unix = 5;
  int32 seconds_remaining = 6;
}

// Server info
message ServerInfoRequest {}

message ServerInfoResponse {
  string server_id = 1;
  string hostname = 2;
  string version = 3;
  string os = 4;
}

// Command listing
message ListCommandsRequest {}

message ListCommandsResponse {
  repeated Command commands = 1;
}

message Command {
  string id = 1;
  string name = 2;
  string description = 3;
  string icon = 4;
  repeated string tags = 5;
  repeated Parameter parameters = 6;
}

message Parameter {
  string name = 1;
  ParameterType type = 2;
  string description = 3;

  // Type-specific fields (optional, depends on type)
  optional int32 min = 4;           // For slider
  optional int32 max = 5;           // For slider
  optional string default_value = 6;
  repeated string options = 7;      // For dropdown
  optional string validation = 8;   // Regex for text
  optional string label_on = 9;     // For toggle
  optional string label_off = 10;   // For toggle
}

enum ParameterType {
  PARAMETER_TYPE_UNSPECIFIED = 0;
  PARAMETER_TYPE_SLIDER = 1;
  PARAMETER_TYPE_TEXT = 2;
  PARAMETER_TYPE_TOGGLE = 3;
  PARAMETER_TYPE_DROPDOWN = 4;
}

// Command execution
message ExecuteCommandRequest {
  string command_id = 1;
  map<string, string> parameters = 2;
}

message ExecuteCommandResponse {
  oneof response {
    string stdout = 1;      // Chunk of stdout
    string stderr = 2;      // Chunk of stderr
    int32 exit_code = 3;    // Final exit code (last message)
    string error = 4;       // Error message if command fails to start
  }

  // Metadata
  optional int64 timestamp_ms = 5;
}
```

---

### API Reference

This section provides detailed documentation for each RPC method in the RemoteControl service.

#### Authentication Requirements

| Method | Authentication | Description |
|--------|---------------|-------------|
| `Enroll` | TLS-only | Available during enrollment before mTLS is established |
| `RequestPairing` | TLS-only | Available during enrollment before mTLS is established |
| `CheckPairingStatus` | TLS-only | Available during enrollment before mTLS is established |
| `ApprovePairing` | **mTLS required** | Server-side operation requiring authenticated access |
| `ListPendingPairings` | **mTLS required** | Server-side operation requiring authenticated access |
| `GetServerInfo` | TLS-only | Public server information, no authentication required |
| `ListCommands` | **mTLS required** | Requires authenticated client certificate |
| `ExecuteCommand` | **mTLS required** | Requires authenticated client certificate |

**Note:** Methods marked "TLS-only" are available over standard TLS connections (typically during the enrollment phase). Methods marked "mTLS required" require the client to present a valid certificate that exists in the server's authorized clients list.

---

#### Enrollment Methods

##### Enroll (QR Code Enrollment)

**Purpose:** Token-based enrollment using a QR code scanned by the Android client.

**Use Case:** Quick pairing when the user has physical access to the PC and can scan a QR code displayed on screen.

**Authentication:** TLS-only (no mTLS required)

**Request:** `EnrollRequest`
- `enrollment_token` (string): One-time UUID token from QR code (expires after use or 5 minutes)
- `client_certificate` (bytes): Full X.509 certificate in DER encoding (ECDSA P-256, self-signed)
- `device_name` (string): User-friendly device name (e.g., "My Pixel 7")

**Response:** `EnrollResponse`
- `success` (bool): True if enrollment succeeded
- `client_id` (string): UUID assigned by server to this client
- `error_message` (string): Error description if success is false

**Behavior:**
1. Server validates enrollment token exists and has not expired
2. Server parses and validates client certificate (must be valid X.509 DER format)
3. Server stores full client certificate in `~/.config/handcontrol/authorized_clients/`
4. Server invalidates the enrollment token (one-time use)
5. Server assigns a new client UUID and returns it to the client
6. Client can now use mTLS with this certificate for all subsequent connections

**Error Scenarios:**
- `PERMISSION_DENIED`: Enrollment token invalid, expired, or already consumed
- `PERMISSION_DENIED`: QR code enrollment disabled in server config
- `INVALID_ARGUMENT`: Invalid certificate format or device_name empty
- `INTERNAL`: Server error storing certificate

**Implementation:** `src/grpc/server.rs:65-134`

---

##### RequestPairing (Approval Mode - Initiate)

**Purpose:** Initiate interactive pairing with verification code to prevent MITM attacks.

**Use Case:** User-friendly pairing via mDNS discovery when camera/QR scanning is not available or when pairing multiple devices.

**Authentication:** TLS-only (no mTLS required)

**Request:** `RequestPairingRequest`
- `device_name` (string): User-friendly device name (e.g., "My Pixel 7")
- `device_model` (string, optional): Device model (e.g., "Google Pixel 7")
- `client_certificate` (bytes): Full X.509 certificate in DER encoding (ECDSA P-256, self-signed)
- `verification_code` (string): 6-digit code computed by client (format: "XXX-XXX")

**Response:** `RequestPairingResponse`
- `pending` (bool): True if request created successfully, false if verification failed
- `pairing_request_id` (string): UUID to poll status with `CheckPairingStatus`
- `timeout_seconds` (int32): How long request remains valid (default 60 seconds)
- `verification_code` (string): Server's computed verification code (must match client's)
- `server_cert_fingerprint` (bytes): SHA256 fingerprint of server certificate
- `error_message` (string): Error description if pending is false

**Behavior:**
1. Server validates approval mode enrollment is enabled in config
2. Server validates client certificate is present and valid
3. **CRITICAL SECURITY:** Server computes verification code using same algorithm as client:
   ```
   client_fp = SHA256(client_certificate_DER)
   server_fp = SHA256(server_certificate_DER)
   code = first_6_digits(SHA256(client_fp || server_fp || server_id))
   ```
4. Server compares client's verification code with its own computed code
5. **If codes match:** Proceed to create pairing request and show notification
6. **If codes DON'T match:** Return error immediately (possible MITM attack)
7. Server creates pairing request with timeout (stored in memory)
8. Server shows OS notification with device name and verification code
9. Server returns pending=true with pairing_request_id for status polling

**Client Validation (Android):**
- Client must verify `pending = true`
- Client must recompute verification code and verify it matches server's response
- Client must verify `server_cert_fingerprint` matches the TLS certificate fingerprint
- Only after these checks pass should client display the verification code UI

**User Action Required:**
- User must visually compare verification codes on phone and PC
- User clicks [Accept] or [Reject] in PC notification
- Android client polls status using `CheckPairingStatus`

**Error Scenarios:**
- `PERMISSION_DENIED`: Approval mode enrollment disabled in server config
- `PERMISSION_DENIED`: Verification code mismatch (possible MITM attack)
- `INVALID_ARGUMENT`: Invalid certificate format or device_name empty
- `INTERNAL`: Server error creating pairing request or showing notification

**Security Properties:**
- Verification code cryptographically binds to actual certificates exchanged
- Out-of-band verification (user visually compares codes)
- Prevents MITM from enrolling by swapping certificates

**Implementation:** `src/grpc/server.rs:136-249`

---

##### CheckPairingStatus (Approval Mode - Poll)

**Purpose:** Poll the status of a pairing request during approval mode enrollment.

**Use Case:** Android client polls this method every 2 seconds while waiting for user approval on the PC.

**Authentication:** TLS-only (no mTLS required)

**Request:** `CheckPairingStatusRequest`
- `pairing_request_id` (string): UUID returned from `RequestPairing`

**Response:** `CheckPairingStatusResponse`
- `status` (PairingStatus enum): Current status of the pairing request
- `client_id` (string): UUID assigned by server (only present if status is APPROVED)
- `error_message` (string): Error description if applicable

**PairingStatus Values:**
- `PAIRING_STATUS_UNSPECIFIED (0)`: Invalid/unknown status
- `PAIRING_STATUS_PENDING (1)`: Waiting for user response
- `PAIRING_STATUS_APPROVED (2)`: User accepted pairing
- `PAIRING_STATUS_REJECTED (3)`: User rejected pairing
- `PAIRING_STATUS_TIMEOUT (4)`: Request expired (default 60 seconds)

**Behavior:**
1. Server looks up pairing request by ID
2. Server checks if request has expired (current time > expires_at)
3. Returns current status based on user action or timeout
4. If status is APPROVED, includes the assigned client_id

**Client Behavior:**
- Poll every 2 seconds with exponential backoff recommended
- Stop polling after receiving APPROVED, REJECTED, or TIMEOUT
- Maximum polling duration should match timeout_seconds from `RequestPairing`

**Error Scenarios:**
- `NOT_FOUND`: Pairing request ID doesn't exist or was already cleaned up
- `INTERNAL`: Server error retrieving pairing request

**Implementation:** `src/grpc/server.rs:251-325`

---

##### ApprovePairing (Approval Mode - Server-side Approval)

**Purpose:** Programmatically approve a pairing request (for CLI tools or automation).

**Use Case:** Server-side CLI command `handcontrol approve <request_id>` to approve pairing without clicking notification.

**Authentication:** **mTLS required** (must be an already-authorized client or server admin)

**Request:** `ApprovePairingRequest`
- `pairing_request_id` (string): UUID of the pending pairing request to approve

**Response:** `ApprovePairingResponse`
- `success` (bool): True if pairing approved successfully
- `client_id` (string): UUID assigned to the newly enrolled client
- `error_message` (string): Error description if success is false

**Behavior:**
1. Server retrieves pairing request by ID
2. Server validates request is still pending (not already approved/rejected/expired)
3. Server stores client certificate from request in authorized_clients
4. Server updates pairing request status to APPROVED
5. Server assigns new client UUID and returns it

**Error Scenarios:**
- `NOT_FOUND`: Pairing request ID doesn't exist
- `FAILED_PRECONDITION`: Request already approved, rejected, or expired
- `INTERNAL`: Server error storing certificate

**CLI Usage:**
```bash
# List pending requests
handcontrol list-pending

# Approve a specific request
handcontrol approve <request_id>
```

**Implementation:** `src/grpc/server.rs:327-370`

---

##### ListPendingPairings (Approval Mode - Management)

**Purpose:** List all pending pairing requests for management and visibility.

**Use Case:** CLI tool or future GUI to show all devices waiting for approval.

**Authentication:** **mTLS required** (must be an already-authorized client or server admin)

**Request:** `ListPendingPairingsRequest` (empty)

**Response:** `ListPendingPairingsResponse`
- `pending_pairings` (repeated PendingPairingInfo): List of all pending requests

**PendingPairingInfo Fields:**
- `request_id` (string): UUID of the pairing request
- `device_name` (string): Name of device requesting pairing
- `device_model` (string): Model of device (may be empty)
- `verification_code` (string): 6-digit verification code for this request
- `expires_at_unix` (int64): Unix timestamp when request expires
- `seconds_remaining` (int32): Seconds until timeout

**Behavior:**
1. Server retrieves all pairing requests with PENDING status
2. Server filters out expired requests
3. Server calculates seconds_remaining for each request
4. Returns list sorted by creation time (oldest first)

**Error Scenarios:**
- `INTERNAL`: Server error retrieving pairing requests

**CLI Usage:**
```bash
# List all pending pairing requests
handcontrol list-pending
```

**Note:** This method is only in the server proto file, not yet in Android proto (see implementation gap).

**Implementation:** `src/grpc/server.rs:372-400`

---

#### Discovery Methods

##### GetServerInfo (Server Discovery)

**Purpose:** Get server metadata and identification information.

**Use Case:** Clients can verify server identity and display server information in the UI.

**Authentication:** TLS-only (no mTLS required, public information)

**Request:** `ServerInfoRequest` (empty)

**Response:** `ServerInfoResponse`
- `server_id` (string): Unique UUID for this server instance
- `hostname` (string): System hostname (e.g., "work-laptop")
- `version` (string): Server version (e.g., "0.1.0")
- `os` (string): Operating system (e.g., "linux", "windows", "macos")

**Behavior:**
1. Server returns its UUID (persistent across restarts, stored in config)
2. Server returns system hostname (via `hostname` crate)
3. Server returns version from CARGO_PKG_VERSION
4. Server returns OS from std::env::consts::OS

**Use Cases:**
- Android displays server name before enrollment
- Verify connecting to correct server
- Debug/support information

**Error Scenarios:**
- Generally does not fail (returns system defaults if needed)

**Implementation:** `src/grpc/server.rs:402-428`

---

#### Command Methods

##### ListCommands (Command Discovery)

**Purpose:** Get list of available commands configured on the server.

**Use Case:** Android client fetches command list after enrollment to populate the UI.

**Authentication:** **mTLS required** (must be enrolled client)

**Request:** `ListCommandsRequest` (empty)

**Response:** `ListCommandsResponse`
- `commands` (repeated Command): List of all configured commands

**Command Fields:**
- `id` (string): Unique command identifier (e.g., "lock-screen")
- `name` (string): Display name (e.g., "Lock Screen")
- `description` (string): Human-readable description
- `icon` (string): Icon identifier (optional)
- `tags` (repeated string): Tags for filtering (e.g., ["media", "audio"])
- `parameters` (repeated Parameter): Command parameters (if any)

**Parameter Fields:**
- `name` (string): Parameter name used in shell command substitution
- `type` (ParameterType): Type of UI control (SLIDER, TEXT, TOGGLE, DROPDOWN)
- `description` (string): Help text for parameter
- `min`, `max` (int32, optional): For SLIDER type
- `default_value` (string, optional): Default value
- `options` (repeated string): For DROPDOWN type
- `validation` (string, optional): Regex validation for TEXT type
- `label_on`, `label_off` (string, optional): For TOGGLE type

**Behavior:**
1. Server reads commands from TOML configuration file
2. Server transforms config format to protobuf Command messages
3. Server maps parameter types (slider, text, toggle, dropdown)
4. Returns all commands with full metadata

**Error Scenarios:**
- `UNAUTHENTICATED`: Client certificate not in authorized list
- `INTERNAL`: Server error reading or parsing config file

**Caching Recommendation:**
- Clients should cache command list
- Re-fetch on connection or periodically (e.g., every 5 minutes)
- Server config changes require client to re-fetch

**Implementation:** `src/grpc/server.rs:430-484`

---

##### ExecuteCommand (Command Execution - Streaming)

**Purpose:** Execute a configured command with real-time streaming output.

**Use Case:** User taps a command in Android app, optionally provides parameters, and receives streaming output.

**Authentication:** **mTLS required** (must be enrolled client)

**Request:** `ExecuteCommandRequest`
- `command_id` (string): ID of command to execute (must exist in config)
- `parameters` (map<string, string>): Parameter name/value pairs

**Response:** `stream ExecuteCommandResponse` (server-streaming)
- `stdout` (string): Chunk of stdout output
- `stderr` (string): Chunk of stderr output
- `exit_code` (int32): Final exit code (sent as last message)
- `error` (string): Error message if command fails to start
- `timestamp_ms` (int64, optional): Unix timestamp in milliseconds

**Behavior:**
1. Server validates command_id exists in config
2. Server validates all required parameters are present
3. Server validates parameter values against schema:
   - Numeric ranges (min/max for sliders)
   - Regex validation (for text inputs)
   - Allowed options (for dropdowns)
4. Server substitutes parameters into shell command with proper escaping
5. Server spawns process:
   - Linux/macOS: `/bin/sh -c "command"`
   - Windows: `cmd.exe /C "command"`
6. Server streams stdout/stderr chunks as they arrive (non-blocking)
7. Server enforces timeout (from config, terminates process if exceeded)
8. Server sends final exit_code as last message
9. Stream closes after exit_code is sent

**Parameter Substitution:**
```
Shell command: "amixer set Master {volume}%"
Parameters: {"volume": "50"}
Result: "amixer set Master 50%"
```

**Security:**
- All parameter values are shell-escaped to prevent injection
- Commands run as the server process user (not root)
- Timeout prevents infinite execution

**Error Scenarios:**
- `UNAUTHENTICATED`: Client certificate not in authorized list
- `NOT_FOUND`: Command ID doesn't exist in config
- `INVALID_ARGUMENT`: Missing required parameters or invalid values
- `DEADLINE_EXCEEDED`: Command execution exceeded timeout
- `INTERNAL`: Server error spawning process

**Streaming Pattern:**
```
Response 1: {stdout: "Starting process...\n", timestamp_ms: 1234567890}
Response 2: {stdout: "Processing...\n", timestamp_ms: 1234567891}
Response 3: {stderr: "Warning: deprecated\n", timestamp_ms: 1234567892}
Response 4: {exit_code: 0, timestamp_ms: 1234567893}
```

**Client Behavior:**
- Display stdout/stderr in real-time (optional)
- Wait for exit_code to determine success/failure
- Handle timeout gracefully (show "Command timed out" message)

**Implementation:** `src/grpc/server.rs:489-620`

---

### Error Handling

**gRPC Status Codes:**
- `UNAUTHENTICATED`: Invalid or missing client certificate (mTLS failure)
- `PERMISSION_DENIED`: Enrollment token invalid/expired, enrollment mode disabled, or verification code mismatch
- `NOT_FOUND`: Command ID or pairing request ID doesn't exist
- `INVALID_ARGUMENT`: Invalid parameters, certificate format, or missing required fields
- `FAILED_PRECONDITION`: Operation not allowed in current state (e.g., approving expired pairing)
- `DEADLINE_EXCEEDED`: Command execution timeout
- `INTERNAL`: Server error (filesystem, parsing, process spawning)

**Error Response Pattern:**
Most methods include an `error_message` field in the response. This provides human-readable error details that can be displayed to the user or logged for debugging.

**Client Error Handling:**
- `UNAUTHENTICATED`: Show "Device unauthorized, please re-enroll"
- `PERMISSION_DENIED`: Show specific error message from response
- `NOT_FOUND`: Show "Command not found, please refresh"
- `INVALID_ARGUMENT`: Show validation error to user
- `DEADLINE_EXCEEDED`: Show "Command timed out"
- `INTERNAL`: Show "Server error, please try again"

---

## User Flows

### Flow 1: Initial Setup (PC)

1. User installs HandControl server
2. Server runs for first time:
   - Creates config directory structure
   - Generates self-signed certificate
   - Creates default `config.toml` with example commands
   - Starts gRPC server
   - Registers mDNS service
3. User edits `config.toml` to add custom commands
4. Server reloads config (hot reload or restart)

### Flow 2A: Device Enrollment - QR Code Mode (Android)

1. User opens HandControl Android app
2. Taps "Add Server" → "Scan QR Code"
3. Scans QR code displayed on PC
4. App validates QR structure
5. Shows server details (name, IP, fingerprint)
6. User confirms pairing
7. App generates client certificate (self-signed X.509, ECDSA P-256, 10-year validity)
8. Private key stored in Android Keystore hardware-backed (never transmitted)
9. Connects to server via TLS, verifies cert fingerprint matches QR
10. Sends `Enroll` RPC with enrollment token and full client certificate
11. Server validates token and stores full client certificate
12. Success: Android pins server cert and shows "Connected to [Server Name]"
13. Failure: Shows error (token expired, network error, etc.)

### Flow 2B: Device Enrollment - Approval Mode with Verification (Android)

1. User opens HandControl Android app
2. App auto-discovers servers via mDNS
3. User sees list of discovered servers (unpaired)
4. User taps server → "Request Pairing"
5. App generates client certificate (self-signed X.509, ECDSA P-256)
6. App computes verification code: `SHA256(SHA256(client_cert) || SHA256(server_cert) || server_id)` → first 6 digits
7. Sends `RequestPairing` RPC with full client certificate and verification code
8. **Server validates verification code (MANDATORY):**
   - Server computes its own verification code using same algorithm
   - **If codes match:** Proceed to step 10 (no MITM)
   - **If codes DON'T match:** Return error immediately, reject pairing (possible MITM attack)
9. **Android validates server response:**
   - Asserts `pending = true` and recomputes the expected verification code
   - Verifies the recomputed code matches the server-provided value
   - Checks `server_cert_fingerprint` matches the TLS certificate fingerprint observed during the request
   - On any mismatch, aborts pairing and shows "Security verification failed" without displaying the code
10. **Both sides display verification code:**
    - **Android:** Shows large verification code (e.g., "482-917")
    - **PC:** Shows notification with device name and same code: "Device 'My Pixel 7' wants to connect. Code: 482-917"
11. **User verifies codes match** (critical security step!)
12. Android shows "Waiting for approval..." with timeout countdown
13. Android polls `CheckPairingStatus` every 2 seconds
14. **If user accepts on PC (after verifying code):**
    - Server stores full client certificate for mTLS, returns `APPROVED`
    - Android pins server cert (TOFU - future connections must use same cert)
    - Android saves client ID
    - Shows "Successfully paired with [Server Name]"
15. **If user rejects on PC (unwanted device):**
    - Server returns `REJECTED`
    - Android shows "Pairing rejected by server"
16. **If timeout (60s):**
    - Server returns `TIMEOUT`
    - Android shows "Pairing request timed out. Please try again."

### Flow 3: Executing Commands

1. User opens app
2. App auto-discovers servers via mDNS (shows list)
3. User selects server
4. App connects via mTLS
5. Fetches command list via `ListCommands` RPC
6. Displays commands grouped by tags
7. User taps command:
   - If no parameters: Execute immediately
   - If parameters: Show UI controls (sliders, text fields, etc.)
8. User confirms/executes
9. App sends `ExecuteCommand` RPC
10. Streams output (shows in UI if needed)
11. Shows completion status (exit code)

### Flow 4: Multiple Devices

1. Each device enrolls independently (separate QR scans)
2. Server tracks all enrolled devices in `metadata.toml`
3. All devices can execute commands concurrently
4. No coordination between devices (stateless)

### Flow 5: Revoking a Device

1. User deletes device's certificate from `authorized_clients/` directory
2. Updates `metadata.toml` to remove entry
3. Device's next connection attempt fails with `UNAUTHENTICATED`
4. Android shows "Device unauthorized, please re-enroll"

---

## UI/UX Requirements

### PC Server

**Initial version:** CLI only
- Logs to stdout/stderr
- Prints QR code to terminal (ASCII art via `qr2term` or similar)
- Shows enrollment events
- Logs command executions
- **Notification system integration** (for approval mode):
  - **Linux:** D-Bus notifications via `notify-rust` crate
  - **Windows:** Windows Toast notifications via `windows` crate
  - **macOS:** Notification Center via `mac-notification-sys` crate
  - Fallback: If notifications unavailable, approval mode auto-disabled

**Future:** GUI with system tray
- Show connected devices
- Generate QR code window
- View command execution history
- Manage authorized devices
- Interactive pairing approval (instead of notifications)

### Android App

**Main Screen:**
```
┌─────────────────────────────────┐
│ HandControl          [⚙️ Settings] │
├─────────────────────────────────┤
│                                 │
│  📱 Discovered Servers          │
│                                 │
│  🖥️  My Desktop PC              │
│      192.168.1.100              │
│      Connected • 3ms            │
│                                 │
│  🖥️  Work Laptop (Unpaired)    │
│      192.168.1.105              │
│      [Request Pairing]          │
│                                 │
│  [➕ Add Server Manually]       │
│  [📷 Scan QR Code]              │
│                                 │
└─────────────────────────────────┘
```

**Pairing Approval Screen (when waiting for PC approval):**
```
┌─────────────────────────────────┐
│ ← Pairing Request               │
├─────────────────────────────────┤
│                                 │
│   🖥️  Work Laptop               │
│   192.168.1.105                 │
│                                 │
│   Verification Code:            │
│   ┏━━━━━━━━━━━━━━━━━━━━━━━━┓   │
│   ┃      482-917           ┃   │
│   ┗━━━━━━━━━━━━━━━━━━━━━━━━┛   │
│                                 │
│   ⚠️  Verify this code matches  │
│   the one shown on your PC      │
│                                 │
│   Waiting for approval...       │
│   ⏱️  Timeout in 45 seconds     │
│                                 │
│         [Cancel]                │
│                                 │
└─────────────────────────────────┘
```

**Command List Screen:**
```
┌─────────────────────────────────┐
│ ← My Desktop PC                 │
├─────────────────────────────────┤
│ 🔍 Search commands...           │
├─────────────────────────────────┤
│ Filters: [All] [Media] [System] │
├─────────────────────────────────┤
│                                 │
│ 🔒 Lock Screen                  │
│    Locks the computer screen    │
│                                 │
│ 🔊 Set Volume                   │
│    Adjust system volume         │
│                                 │
│ 🔇 Toggle Mute                  │
│    Mute/unmute audio            │
│                                 │
└─────────────────────────────────┘
```

**Command Execution (with parameters):**
```
┌─────────────────────────────────┐
│ ← Set Volume                    │
├─────────────────────────────────┤
│                                 │
│  Volume level (0-100)           │
│                                 │
│  ●────────────○─────────────    │
│  0           50            100  │
│                                 │
│  Current: 50                    │
│                                 │
│         [Execute Command]       │
│                                 │
└─────────────────────────────────┘
```

**QR Code Scanner:**
- Full-screen camera view
- Shows server details after scan
- Confirmation dialog before enrollment

---

## Platform-Specific Considerations

### Linux
- Shell: `/bin/sh` (POSIX compatible)
- Config: `~/.config/handcontrol/`
- Systemd integration (optional): Run as user service

### Windows
- Shell: `cmd.exe` or `powershell.exe` (configurable)
- Config: `%APPDATA%\handcontrol\`
- Handle path separators (`\` vs `/`)
- Windows Service support (optional)

### macOS
- Shell: `/bin/sh`
- Config: `~/Library/Application Support/handcontrol/`
- launchd integration (optional)

---

## Validation & Error Handling

### Server-Side Validation

**Command Execution:**
- Parameter types match schema
- Required parameters present
- Numeric ranges respected (min/max for sliders)
- Text validation regex passes
- Shell command exists (optional check)

**Error Responses:**
- Parameter validation failed: Return `INVALID_ARGUMENT` with details
- Command not found: Return `NOT_FOUND`
- Timeout: Return `DEADLINE_EXCEEDED`
- Execution error: Stream stderr, return non-zero exit code

### Client-Side Validation

**Before sending:**
- Validate parameter inputs locally
- Show validation errors in UI
- Disable "Execute" button until valid

**Connection errors:**
- Server unreachable: Show "Server offline" banner
- Certificate mismatch: Show "Security error, please re-enroll"
- Network timeout: Show retry button

---

## Logging

### Server Logs

**Location:**
- stdout/stderr (systemd captures to journal)
- Optional: `~/.local/state/handcontrol/handcontrol.log`

**Log Levels:**
- `ERROR`: Critical failures
- `WARN`: Recoverable issues (invalid client, command timeout)
- `INFO`: Enrollment, command execution, connection events
- `DEBUG`: Detailed protocol messages

**Log Format:**
```
[2025-10-29T14:30:00Z INFO] Server started on 0.0.0.0:50051
[2025-10-29T14:30:05Z INFO] QR enrollment request from device "My Pixel 7"
[2025-10-29T14:30:05Z INFO] Client enrolled via QR: client_id=abc-123
[2025-10-29T14:30:45Z INFO] Approval pairing request from device "My Phone"
[2025-10-29T14:30:50Z INFO] Pairing approved by user, client enrolled: client_id=def-456
[2025-10-29T14:31:20Z WARN] Pairing rejected by user: device="Suspicious Device"
[2025-10-29T14:31:45Z ERROR] Pairing verification failed: device="Unknown Device" (possible MITM)
[2025-10-29T14:35:22Z INFO] Command executed: command_id=lock-screen, client_id=abc-123, exit_code=0
[2025-10-29T14:36:10Z WARN] Command timeout: command_id=slow-script, client_id=abc-123
```

**Important:** Verification codes MUST NOT be logged (sensitive security data)

### Android Logs

- Use Android Logcat (standard logging)
- Log connection events, RPC calls, errors
- No PII in logs (no tokens, certs)

---

## Security Considerations

### Threat Model

**Trusted:**
- User's local network
- Physical access to PC

**Mitigated:**
- **Man-in-the-middle (QR mode):** Certificate fingerprint in QR code prevents MITM
- **Man-in-the-middle (Approval mode):** Verification code prevents MITM at first connection
- **Unauthorized devices:** Enrollment token (QR) or user approval (interactive) + mTLS prevents rogue clients
- **Command injection:** Parameter escaping prevents shell injection
- **Token replay:** One-time enrollment tokens (QR mode)
- **Pairing request replay:** Single-use pairing request IDs (Approval mode)

**Not Mitigated (Acceptable):**
- **Physical access to PC:** Attacker with filesystem access can steal server key
- **Compromised Android device:** Attacker can execute commands (same as legitimate user)
- **Network sniffing on other subnets:** Out of scope (local network only)

### Verification Code Security (Approval Mode)

**How it prevents MITM attacks:**

1. **The Problem:** During first connection, Android doesn't know the server's real certificate yet. An attacker could intercept the connection and present their own certificate.

2. **The Solution:** Both sides compute a verification code from the actual certificates being exchanged:
   ```
   # Both sides use the SAME algorithm
   client_fingerprint = SHA256(client_certificate_DER)
   server_fingerprint = SHA256(server_certificate_DER)
   code_material = SHA256(client_fingerprint || server_fingerprint || server_id)
   verification_code = first_6_digits(code_material)
   # Format: "482-917" (XXX-XXX)
   ```

   **Android computes:**
   ```
   client_fp = SHA256(my_certificate)
   server_fp = SHA256(received_server_certificate)  # ← Could be MITM's cert!
   code_A = first_6_digits(SHA256(client_fp || server_fp || server_id))
   ```

   **Server computes:**
   ```
   client_fp = SHA256(received_client_certificate)
   server_fp = SHA256(my_certificate)
   code_S = first_6_digits(SHA256(client_fp || server_fp || server_id))
   ```

3. **If no MITM:** Both codes match (e.g., "482-917") because they used the same certificates
4. **If MITM present:** Codes differ because:
   - Android computed code using MITM's server certificate
   - Server computed code using real server certificate
   - Different certificates → different fingerprints → different codes
5. **User verification:** By checking codes match, user confirms they're seeing the real server's certificate

**Why this works:**
- Code is cryptographically bound to the actual certificates exchanged
- Attacker can't forge the server's code without the server's private key
- User provides "out-of-band" verification (visual comparison)
- Similar to SSH key fingerprints or Bluetooth pairing codes

**After first connection:**
- Android pins the server certificate (TOFU - Trust On First Use)
- Future connections verify against pinned cert
- MITM becomes impossible even without verification codes

### Best Practices

- **Never log sensitive data:** Tokens, certificate private keys, verification codes
- **Sanitize command parameters:** Escape shell special characters
- **Validate all inputs:** Server validates all RPC parameters
- **Principle of least privilege:** Commands run as server user (not root)
- **Audit trail:** Log all command executions with client ID
- **Verification code validation:** Server should verify client's code matches before accepting pairing

---

## Performance Requirements

### Server
- **Command execution:** Start within 100ms
- **Streaming output:** <50ms latency per chunk
- **Concurrent clients:** Support 10+ simultaneous connections
- **Memory:** <50MB base, +10MB per active connection
- **CPU:** Minimal when idle, <5% during command execution

### Android
- **mDNS discovery:** Complete within 5 seconds
- **Command list fetch:** <500ms on local network
- **Command execution:** Acknowledge within 100ms
- **Battery:** Minimal drain (no persistent connection unless app open)

---

## Testing Strategy

### Server Tests
- **Unit tests:** Command parsing, parameter substitution, validation
- **Integration tests:** gRPC endpoints, mTLS handshake, enrollment flow
- **Security tests:** Certificate validation, token expiry, injection attempts

### Android Tests
- **Unit tests:** Parameter validation, QR parsing
- **UI tests:** Command list, parameter inputs, execution flow
- **Integration tests:** End-to-end with mock server

### Manual Testing
- Multi-device enrollment
- Concurrent command execution
- Network interruption recovery
- Certificate revocation

---

## Future Enhancements (Not in V1)

- Command history (executed commands log)
- Favorites/pinned commands
- Command macros (chain multiple commands)
- Server-side GUI
- Import/export configs
- Command templates/marketplace
- Bi-directional control (PC → Android notifications)
- Widget support (quick commands from home screen)
- Voice commands integration
- Cloud relay (for remote access outside LAN)

---

## Open Questions

1. Should the server support dynamic config reload, or require restart?
2. Should command output be persisted on server side (for review)?
3. Maximum command output size (to prevent memory exhaustion)?
4. Should Android support multiple servers simultaneously, or one at a time?
5. Icon system: Built-in icon set, or custom icon URLs?

---

## Appendix

### Example Commands

**Linux:**
```toml
[[command]]
id = "lock"
shell = "loginctl lock-session"

[[command]]
id = "suspend"
shell = "systemctl suspend"

[[command]]
id = "screenshot"
shell = "scrot ~/Pictures/screenshot-$(date +%Y%m%d-%H%M%S).png"

[[command]]
id = "brightness"
shell = "brightnessctl set {level}%"
[[command.parameters]]
name = "level"
type = "slider"
min = 10
max = 100
```

**Windows:**
```toml
[[command]]
id = "lock"
shell = "rundll32.exe user32.dll,LockWorkStation"

[[command]]
id = "shutdown"
shell = "shutdown /s /t 0"

[[command]]
id = "volume"
shell = "nircmd.exe setsysvolume {level}"
[[command.parameters]]
name = "level"
type = "slider"
min = 0
max = 65535
```

**macOS:**
```toml
[[command]]
id = "lock"
shell = "/System/Library/CoreServices/Menu Extras/User.menu/Contents/Resources/CGSession -suspend"

[[command]]
id = "volume"
shell = "osascript -e 'set volume output volume {level}'"
[[command.parameters]]
name = "level"
type = "slider"
min = 0
max = 100
```

### Dependencies

**Rust Crates:**
```toml
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

# Platform-specific notification support (for approval mode)
[target.'cfg(target_os = "linux")'.dependencies]
notify-rust = "4"

[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "0.58", features = ["UI.Notifications"] }

[target.'cfg(target_os = "macos")'.dependencies]
mac-notification-sys = "0.6"

[build-dependencies]
tonic-build = "0.12"
```

**Android (Gradle):**
```kotlin
dependencies {
    implementation("io.grpc:grpc-kotlin-stub:1.4.1")
    implementation("io.grpc:grpc-okhttp:1.65.1")
    implementation("com.google.protobuf:protobuf-kotlin:4.27.1")
    implementation("androidx.compose.ui:ui:1.6.8")
    implementation("androidx.compose.material3:material3:1.2.1")
}
```

---

## Summary of Key Features

### Dual Enrollment Modes

**QR Code Mode:**
- Fast, no interaction required
- Perfect for initial setup with physical access
- One-time token in QR code
- Certificate fingerprint prevents MITM
- Immediate pairing after scan

**Approval Mode (with Verification Code):**
- User-friendly, no QR scanning needed
- Discover servers via mDNS
- Interactive approval via OS notifications
- **6-digit verification code** prevents MITM attacks
- User verifies code matches on both devices (like Bluetooth pairing)
- Great for remote pairing or multiple devices

Both modes can be enabled/disabled independently in server config, giving users flexibility based on their use case.

### Security Comparison

| Aspect | QR Code Mode | Approval Mode |
|--------|--------------|---------------|
| MITM Protection | Certificate fingerprint in QR | Verification code (user verifies) |
| User Effort | Scan QR code | Check 6-digit code |
| Physical Access | Required (to see QR) | Not required (see notification) |
| Setup Speed | Instant | ~5-10 seconds (wait for approval) |
| Multi-device | Generate QR for each device | Just tap "pair" on each device |
| Security Model | Pre-authenticated (token in QR) | User-authenticated (approval + code) |

Both modes are equally secure when used correctly. The verification code in approval mode provides the same MITM protection as the certificate fingerprint in QR mode.

---

**Document Status:** Ready for review and implementation planning.
