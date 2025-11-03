# Product Requirements Document: HandControl CLI/TUI Client

**Version:** 1.0
**Date:** 2025-11-02
**Status:** Draft

## Overview

HandControl CLI/TUI Client extends the HandControl remote command execution system to terminal-based environments. It provides two complementary interfaces:

- **CLI (Command-Line Interface)**: Non-interactive commands for scripting, automation, and integration with shell workflows
- **TUI (Terminal User Interface)**: Interactive terminal application for power users who prefer keyboard-driven workflows

Both clients communicate with HandControl servers using the same gRPC protocol and **end-to-end encryption model** as the Android client: TLS is negotiated directly between the CLI/TUI client and the managed server, while intermediate relays forward opaque bytes.

## Goals

- Provide terminal-based access to HandControl servers for power users
- Enable automation and scripting scenarios with non-interactive CLI commands
- Support users without Android devices who need a desktop client
- Maintain security parity with Android client (mTLS, verification codes)
- Integrate seamlessly with Unix/shell workflows (pipes, fzf, grep, scripts)
- Cross-platform support (Linux, macOS, Windows)

## Non-Goals

- GUI/desktop application (use Android or future desktop GUI)
- Server functionality (that's the main `handcontrol` server binary)
- Mobile device support (use Android client)
- Web-based interface
- Graphical QR code scanning (Phase 1 - may add webcam support later)

---

## Architecture

### Binary Structure

**Two separate binaries** with a shared library:

```
handcontrol/
├── cli/                      # CLI client crate
│   ├── Cargo.toml
│   └── src/
│       └── main.rs           # handcontrol-cli binary
│
├── tui/                      # TUI client crate
│   ├── Cargo.toml
│   └── src/
│       └── main.rs           # handcontrol-tui binary
│
└── client-lib/               # Shared client library
    ├── Cargo.toml
    └── src/
        ├── lib.rs
        ├── discovery/        # mDNS discovery
        ├── enrollment/       # QR and approval enrollment
        ├── grpc_client/      # gRPC client with mTLS
        ├── certificates/     # Certificate management
        ├── storage/          # Config and credential storage
        └── commands/         # Command execution logic
```

### Rationale for Separate Binaries

**Pros:**
- Smaller binary size for CLI-only users
- Clear separation of concerns (non-interactive vs interactive)
- Independent versioning and release cycles if needed
- Easier to optimize each for its use case
- CLI can be distributed alone for minimal installations

**Cons:**
- Slightly more code duplication (mitigated by shared library)
- Two binaries to maintain

**Decision:** The benefits outweigh the costs, especially for automation scenarios where binary size and startup time matter.

---

## Technology Stack

### Shared Client Library (`client-lib`)

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | 1.48.0 | Async runtime |
| `tonic` | 0.14.2 | gRPC client |
| `prost` | 0.14.1 | Protobuf encoding |
| `rustls` | 0.23.34 | mTLS implementation (end-to-end TLS; relays never decrypt data) |
| `rcgen` | 0.14.5 | Client certificate generation (ECDSA P-256) |
| `mdns-sd` | 0.15.1 | mDNS discovery |
| `serde` | 1.0.228 | Config serialization |
| `toml` | 0.9.8 | Config file parsing |
| `directories` | 6.0.0 | Platform-specific paths |
| `anyhow` | 1.0.100 | Error handling |
| `tracing` | 0.1.41 | Logging |

### CLI Binary (`handcontrol-cli`)

| Crate | Version | Purpose |
|-------|---------|---------|
| `clap` | 4.5.0 | Argument parsing with derive macros |
| `handcontrol-client-lib` | 0.1.0 | Shared client functionality |

### TUI Binary (`handcontrol-tui`)

| Crate | Version | Purpose |
|-------|---------|---------|
| `ratatui` | 0.28.1 | Terminal UI framework |
| `crossterm` | 0.28.1 | Terminal backend |
| `tokio` | 1.48.0 | Async runtime (streaming) |
| `handcontrol-client-lib` | 0.1.0 | Shared client functionality |

---

## Security Model

### End-to-End TLS & Certificate Management

**TLS Termination: End-to-End.**
- CLI/TUI establishes a mutually-authenticated TLS session directly with the managed server.
- Relay tunnels forward the encrypted byte stream without decrypting or re-encrypting traffic.
- Relay access to telemetry only includes tunnel metadata (sizes, timing), never plaintext RPC payloads.

**Client Certificates:**
- Algorithm: ECDSA P-256 (same as Android)
- Generation: Using `rcgen` crate
- Storage: `~/.config/handcontrol/client-certs/<server_id>/`
  - `client.crt` - Certificate (PEM)
  - `client.key` - Private key (0600)
  - `server.crt.pinned` - SHA256 fingerprint of server cert
- Permissions: 0600 for private keys, 0644 for certificates

**Server Certificate Pinning:**
- Trust On First Use: first connection records the SHA256 fingerprint.
- Every handshake re-validates against the pinned fingerprint before completing.
- Any mismatch aborts the session and surfaces an actionable error (prevents MITM or relay tampering).

**Verification Code:**
- Algorithm matches Android:
  ```
  code = SHA256(
    SHA256(client_cert) ||
    SHA256(server_cert) ||
    server_id ||
    random_nonce
  )
  first_6_digits(code) formatted as "XXX-XXX"
  ```

- Server includes the 32-byte `verification_nonce` in pairing responses.
- CLI/TUI recomputes the displayed code using that nonce, ensuring both sides show the same value while all sensitive material remains inside the end-to-end TLS channel.

### Enrollment Modes

Both QR Code and Approval modes supported (same as Android):

**QR Code Mode:**
1. Server displays QR as ASCII art in terminal
2. User copies enrollment data (JSON) to client
3. Client validates cert fingerprint
4. Client generates certificate and enrolls

**Approval Mode:**
1. Client discovers server via mDNS
2. Client requests pairing with device metadata (initial code field left empty)
3. Server responds with `verification_code` and `verification_nonce`
4. Client recomputes the code locally using the nonce and displays it
5. User verifies codes match
6. User approves on server (notification or CLI command)
7. Client polls status and completes enrollment

---

## CLI Mode (`handcontrol-cli`)

### Design Principles

- Non-interactive by default
- Output suitable for parsing (tab-separated, JSON optional)
- Exit codes indicate success/failure
- Compatible with Unix pipes and tools (grep, awk, fzf, jq)
- Idempotent operations where possible
- Mirror common CLI conventions: surface command stdout/stderr by default, `--quiet` to suppress, `--stream` for live tails

### Command Reference

#### Discovery

```bash
# Discover servers via mDNS
handcontrol-cli discover [--timeout=5] [--json]

# Output (tab-separated):
# HOSTNAME    IP              PORT    SERVER_ID                             STATUS
# my-desktop  192.168.1.100   50051   550e8400-e29b-41d4-a716-446655440000  enrolled
# work-laptop 192.168.1.105   50051   abc12345-e29b-41d4-a716-446655440001  available

# List enrolled servers
handcontrol-cli list-servers [--json]

# Output:
# SERVER_ID                             HOSTNAME    IP              LAST_SEEN
# 550e8400-e29b-41d4-a716-446655440000  my-desktop  192.168.1.100   2025-11-02T10:30:00Z
```

#### Enrollment

```bash
# Enroll via QR code (paste JSON payload from server)
handcontrol-cli enroll qr [--server-id=<id>] [--payload=<json>]
# Interactive: prompts for QR payload if not provided

# Request approval-based pairing
handcontrol-cli enroll approve <server-id-or-hostname> \
  --device-name="My Laptop" \
  [--device-model="ThinkPad X1"]

# Output:
# Verification code: 482-917
# (Server nonce received and stored; code recomputed locally before display)
# Waiting for approval (timeout: 60s)...
# [After approval]
# Successfully enrolled to server: my-desktop
# Client ID: def45678-e29b-41d4-a716-446655440002

# Show server info (works before enrollment)
handcontrol-cli info <server-id-or-hostname>

# Output:
# Server ID:    550e8400-e29b-41d4-a716-446655440000
# Hostname:     my-desktop
# Version:      0.1.0
# OS:           linux
# Enrolled:     yes
# Client ID:    def45678-e29b-41d4-a716-446655440002
```

#### Commands

```bash
# List available commands
handcontrol-cli list <server-id-or-hostname> [--tag=media] [--json]

# Output (tab-separated):
# ID              NAME            DESCRIPTION                 TAGS
# lock-screen     Lock Screen     Locks the computer screen   system,security
# set-volume      Set Volume      Adjust system volume        media,audio

# Execute command (non-interactive)
handcontrol-cli exec <server> <command-id> [key=value ...]

# Examples:
handcontrol-cli exec my-desktop lock-screen
handcontrol-cli exec my-desktop set-volume level=50
handcontrol-cli exec my-desktop toggle-mute muted=true

# With live streaming (prints each chunk as it arrives, prefixed with channel):
handcontrol-cli exec my-desktop long-script --stream
# STDOUT Starting process...
# STDOUT Processing item 1...
# STDERR Warning: deprecated API
# STDOUT Done!
# EXIT 0

# Default buffered mode (stdout/stderr replayed after command finishes):
handcontrol-cli exec my-desktop lock-screen
# Locks the computer screen
# (use --quiet to suppress buffered output)

# Suppress all command output (still respects exit codes):
handcontrol-cli exec my-desktop lock-screen --quiet
```

#### Management

```bash
# Remove enrollment
handcontrol-cli remove <server-id-or-hostname> [--confirm]

# Show client configuration
handcontrol-cli config [--show-secrets]

# Update client configuration
handcontrol-cli config set <key-path> <value>
# Examples:
handcontrol-cli config set discovery.timeout_seconds 10
handcontrol-cli config set discovery.prefer_ipv6 true
```
Keys follow dotted TOML paths (section.key) to align with how tools like `git config` and `kubectl config` address nested settings.

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Command not found |
| 3 | Server not found or unreachable |
| 4 | Authentication failed (enrollment required) |
| 5 | Certificate validation failed |
| 6 | Timeout |
| 7 | Invalid arguments |
| 8 | Command execution failed (non-zero exit code) |

### Output Formats

**Default: Tab-separated values**
- Easy to parse with `awk`, `cut`, `column`
- Compatible with `fzf` for selection
- Command output is buffered and replayed after completion (use `--stream` for live output or `--quiet` to suppress)

**JSON mode (`--json` flag):**
- Structured output for programmatic parsing
- Includes additional metadata

**Example with fzf:**
```bash
# Select server interactively
SERVER=$(handcontrol-cli list-servers | fzf | cut -f1)

# Select and execute command
CMD=$(handcontrol-cli list $SERVER | fzf | cut -f1)
handcontrol-cli exec $SERVER $CMD
```

---

## TUI Mode (`handcontrol-tui`)

### Design Principles

- Keyboard-driven navigation (Vim-like keybindings)
- Real-time updates for discovery and streaming output
- Multi-pane layout for context
- Responsive to terminal resize
- Async operations don't block UI

### Layout

```
┌─────────────────────────────────────────────────────────────────────────┐
│ HandControl TUI v0.1.0                              [q] Quit [?] Help  │
├─────────────────────┬───────────────────────────────────────────────────┤
│  Servers            │  Commands (my-desktop)                            │
│                     │                                                   │
│  > my-desktop       │  [system]                                         │
│      192.168.1.100  │  > lock-screen    Locks the computer screen       │
│      Enrolled       │    suspend        Suspend the computer            │
│                     │                                                   │
│    work-laptop      │  [media]                                          │
│      192.168.1.105  │    set-volume     Adjust system volume            │
│      Available      │    toggle-mute    Mute/unmute audio               │
│                     │    switch-output  Switch audio output             │
│  [d] Discover       │                                                   │
│  [e] Enroll         │  [/] Search  [t] Filter by tag  [x] Execute      │
│  [r] Remove         │                                                   │
└─────────────────────┴───────────────────────────────────────────────────┘
│  Status: Ready                                           Latency: 3ms  │
└─────────────────────────────────────────────────────────────────────────┘
```

### Key Views

#### 1. Server List View (default)

- Left pane: Servers (discovered + enrolled)
- Right pane: Commands for selected server
- Bottom bar: Status and help

**Navigation:**
- `j/k` or arrow keys: Move selection
- `Enter`: Select server / execute command
- `d`: Trigger discovery
- `e`: Enroll to selected server
- `r`: Remove enrollment
- `Tab`: Switch pane
- `q`: Quit

#### 2. Enrollment View

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Enroll to Server: work-laptop                                           │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  Choose enrollment method:                                               │
│                                                                          │
│  > [Q] QR Code Enrollment                                               │
│      Paste QR code data from server                                     │
│                                                                          │
│    [A] Approval-based Pairing                                           │
│      Request approval with verification code                            │
│                                                                          │
│  [Esc] Cancel                                                           │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

**Approval pairing flow:**

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Pairing Request Pending                                                 │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  Server: work-laptop (192.168.1.105)                                    │
│                                                                          │
│  Verification Code:                                                     │
│  ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓  │
│  ┃                          482-917                                  ┃  │
│  ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛  │
│                                                                          │
│  Verify this code matches the one shown on the server.                  │
│                                                                          │
│  Waiting for approval... (45 seconds remaining)                         │
│                                                                          │
│  [Esc] Cancel                                                           │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

#### 3. Command Execution View

For commands with parameters:

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Execute Command: set-volume                                             │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  Adjust system volume                                                   │
│                                                                          │
│  Parameters:                                                            │
│                                                                          │
│  level (0-100):                                                         │
│  [████████████████░░░░░░░░░░░░░░░░] 50                                  │
│  ← → to adjust, or type number: [50__]                                  │
│                                                                          │
│                                                                          │
│  [Enter] Execute  [Esc] Cancel                                          │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

**Streaming output:**

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Executing: long-script                                              [×] │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  10:30:01.234  Starting process...                                      │
│  10:30:02.123  Processing item 1 of 100...                              │
│  10:30:03.456  Processing item 2 of 100...                              │
│  10:30:04.789  Warning: deprecated API                        [STDERR]  │
│  10:30:05.012  Processing item 3 of 100...                              │
│  ...                                                                     │
│                                                                          │
│  ▼ Scroll: j/k  [Space] Pause/Resume  [Esc] Close                      │
│                                                                          │
├─────────────────────────────────────────────────────────────────────────┤
│  Running... (5.2s elapsed)                                              │
└─────────────────────────────────────────────────────────────────────────┘
```

#### 4. Settings View

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Settings                                                                │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  Discovery                                                              │
│  > Auto-discover on startup        [×] Enabled                          │
│    Discovery timeout               [5_] seconds                         │
│    Prefer IPv6                     [×] Enabled                          │
│                                                                          │
│  Connection                                                             │
│    Direct connection timeout       [5_] seconds                         │
│    Command execution timeout       [300] seconds                        │
│                                                                          │
│  Display                                                                │
│    Show timestamps in output       [×] Enabled                          │
│    Color scheme                    [Default ▼]                          │
│                                                                          │
│  [Enter] Edit  [s] Save  [Esc] Cancel                                   │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### Keybindings

**Global:**
- `q`: Quit application
- `?`: Show help
- `:`: Command mode (future)
- `Esc`: Cancel / Go back

**Server List:**
- `j/k` or `↓/↑`: Navigate servers
- `Enter`: Select server
- `d`: Trigger discovery
- `e`: Enroll to selected server
- `r`: Remove enrollment
- `i`: Show server info
- `Tab`: Switch to command pane

**Command List:**
- `j/k` or `↓/↑`: Navigate commands
- `Enter`: Execute command
- `/`: Search commands
- `t`: Filter by tag
- `Tab`: Switch to server pane

**Command Execution:**
- `Space`: Pause/resume streaming output
- `j/k` or `↓/↑`: Scroll output
- `Esc`: Close (if finished) or cancel (if running)

---

## Parameter Handling

Both CLI and TUI must support all parameter types from the protocol:

### 1. Slider (Numeric Range)

**Config:**
```toml
[[command.parameters]]
name = "volume"
type = "slider"
min = 0
max = 100
default = 50
step = 5
```

**CLI:**
```bash
handcontrol-cli exec server set-volume volume=75
# Validates: 0 <= 75 <= 100
```

**TUI:**
- Interactive slider bar
- Arrow keys to adjust
- Direct numeric input

### 2. Text

**Config:**
```toml
[[command.parameters]]
name = "message"
type = "text"
default = "Hello"
validation = "^[a-zA-Z0-9 ]+$"
```

**CLI:**
```bash
handcontrol-cli exec server send-message message="Hello World"
# Validates against regex
```

**TUI:**
- Text input field
- Live validation feedback
- Error message if invalid

### 3. Toggle (Boolean)

**Config:**
```toml
[[command.parameters]]
name = "muted"
type = "toggle"
default = true
label_on = "Mute"
label_off = "Unmute"
```

**CLI:**
```bash
handcontrol-cli exec server toggle-mute muted=true
# Accepts: true/false, yes/no, 1/0, on/off (normalized to true/false before sending)
```
CLI normalization aligns with the current server validator, which requires the literal strings `true` or `false`.

**TUI:**
- Checkbox or toggle switch
- Space to toggle

### 4. Dropdown (Selection)

**Config:**
```toml
[[command.parameters]]
name = "output"
type = "dropdown"
options = ["speakers", "headphones", "hdmi"]
default = "speakers"
```

**CLI:**
```bash
handcontrol-cli exec server switch-output output=headphones
# Validates: value in options[]
```

**TUI:**
- Dropdown menu
- Arrow keys to select
- Enter to confirm

---

## Configuration

### Client Config File

**Location:**
- Linux: `~/.config/handcontrol/client.toml`
- macOS: `~/Library/Application Support/handcontrol/client.toml`
- Windows: `%APPDATA%\handcontrol\client.toml`

**Schema:**

```toml
[discovery]
auto_discover = true          # Auto-discover on TUI startup
timeout_seconds = 5           # mDNS discovery timeout
prefer_ipv6 = true            # Prefer IPv6 addresses when available
include_link_local = false    # Include link-local addresses

[connection]
timeout_seconds = 5           # Connection timeout
command_timeout_seconds = 300 # Command execution timeout
retry_attempts = 3            # Connection retry attempts
retry_delay_ms = 1000         # Delay between retries

[tui]
show_timestamps = true        # Show timestamps in output
color_scheme = "default"      # Color scheme (default, dark, light)
auto_scroll = true            # Auto-scroll command output
confirm_commands = false      # Confirm before executing (unless command requires it)

[cli]
output_format = "tsv"         # Default output format (tsv, json)
show_headers = true           # Show headers in TSV output
color_output = "auto"         # Color output (auto, always, never)

# Optional: Override default device name
[device]
name = "My Laptop"
model = "ThinkPad X1 Carbon"
```

### Enrolled Servers Registry

**Location:** `~/.config/handcontrol/servers.toml`

```toml
[[server]]
id = "550e8400-e29b-41d4-a716-446655440000"
hostname = "my-desktop"
ip = "192.168.1.100"
port = 50051
enrolled_at = "2025-11-02T10:30:00Z"
last_seen = "2025-11-02T14:22:00Z"
client_id = "def45678-e29b-41d4-a716-446655440002"
cert_fingerprint = "SHA256:abc123..."  # Server cert fingerprint (pinned)
cert_path = "client-certs/550e8400-e29b-41d4-a716-446655440000/"

[[server]]
id = "abc12345-e29b-41d4-a716-446655440001"
hostname = "work-laptop"
ip = "192.168.1.105"
port = 50051
enrolled_at = "2025-11-01T09:15:00Z"
last_seen = "2025-11-02T12:00:00Z"
client_id = "ghi78901-e29b-41d4-a716-446655440003"
cert_fingerprint = "SHA256:xyz789..."
cert_path = "client-certs/abc12345-e29b-41d4-a716-446655440001/"
```

### Certificate Storage Structure

```
~/.config/handcontrol/
├── client.toml                    # Client configuration
├── servers.toml                   # Enrolled servers registry
└── client-certs/                  # Certificate storage
    ├── 550e8400-.../              # Server ID
    │   ├── client.crt             # Client certificate (PEM) [0644]
    │   ├── client.key             # Client private key (PEM) [0600]
    │   └── server.crt.pinned      # Pinned server cert fingerprint [0644]
    └── abc12345-.../
        ├── client.crt
        ├── client.key
        └── server.crt.pinned
```

**Security:**
- All `client.key` files: permissions 0600 (readable only by owner)
- Certificates and config: permissions 0644
- Parent directory: permissions 0700

---

## User Workflows

### Workflow 1: Initial Setup (CLI)

```bash
# 1. Install client
cargo install handcontrol-cli

# 2. Discover servers
handcontrol-cli discover

# 3. Enroll to server (approval mode)
handcontrol-cli enroll approve my-desktop --device-name="My Laptop"
# Output: Verification code: 482-917
# (User verifies code matches on server, approves)

# 4. List available commands
handcontrol-cli list my-desktop

# 5. Execute command
handcontrol-cli exec my-desktop lock-screen
```

### Workflow 2: Interactive Usage (TUI)

```bash
# 1. Launch TUI
handcontrol-tui

# 2. Auto-discovery runs, shows server list

# 3. Navigate to server, press 'e' to enroll

# 4. Choose approval method, verify code

# 5. Browse commands, select with Enter

# 6. Fill parameters (if any), execute

# 7. Watch streaming output
```

### Workflow 3: Scripting (CLI)

```bash
#!/bin/bash
# Example: Lock all enrolled servers

for server_id in $(handcontrol-cli list-servers | tail -n +2 | cut -f1); do
  echo "Locking $server_id..."
  handcontrol-cli exec "$server_id" lock-screen || echo "Failed to lock $server_id"
done
```

### Workflow 4: Integration with fzf (CLI)

```bash
# Interactive server and command selection
select_and_run() {
  SERVER=$(handcontrol-cli list-servers | fzf --header="Select server" | cut -f1)
  [ -z "$SERVER" ] && return

  CMD=$(handcontrol-cli list "$SERVER" | fzf --header="Select command" | cut -f1)
  [ -z "$CMD" ] && return

  handcontrol-cli exec "$SERVER" "$CMD" --stream
}

# Add to .bashrc or .zshrc
alias hc-run=select_and_run
```

---

## Implementation Phases

### Phase 1: Basic CLI (MVP)

**Goal:** Functional CLI for core workflows

**Features:**
- ✅ mDNS discovery
- ✅ Enrollment (QR code via copy/paste, approval mode)
- ✅ List servers and commands
- ✅ Execute commands (all parameter types)
- ✅ Certificate generation and storage
- ✅ Server certificate pinning
- ✅ Basic configuration file

**Deliverables:**
- `handcontrol-cli` binary
- `handcontrol-client-lib` library crate
- Unit tests for core functionality
- README with usage examples

**Timeline:** 2-3 weeks

### Phase 2: Full TUI

**Goal:** Interactive terminal UI for power users

**Features:**
- ✅ Server list view with discovery
- ✅ Command browser with search/filter
- ✅ Parameter input forms (all types)
- ✅ Real-time streaming output
- ✅ Settings panel
- ✅ Keyboard navigation

**Deliverables:**
- `handcontrol-tui` binary
- Comprehensive keybinding documentation
- Demo video/GIF

**Timeline:** 3-4 weeks

### Phase 3: Advanced Features

**Goal:** Feature parity with Android client

**Features:**
- ✅ Config hot-reload (watch `config_version`)
- ✅ Dynamic parameter defaults (shell command execution)
- ✅ Relay support (fallback to relay server)
- ✅ Command history in TUI
- ✅ Favorites/pinned commands
- ✅ Multi-server command execution (CLI)

**Deliverables:**
- Updated binaries with new features
- Performance optimizations
- Integration tests

**Timeline:** 2-3 weeks

### Phase 4: Polish & Future Enhancements (Optional)

**Features:**
- Webcam QR code scanning (using `quirc` + `rscam`)
- OS keyring integration (using `keyring-rs`)
- Command output history/logs
- Export/import server configurations
- Shell completion scripts (bash, zsh, fish)
- Man pages

**Timeline:** Ongoing

---

## Testing Strategy

### Unit Tests

**Client Library (`client-lib`):**
- Certificate generation (ECDSA P-256)
- Verification code computation
- Parameter validation (all types)
- Config parsing
- Storage operations

**CLI:**
- Argument parsing
- Output formatting (TSV, JSON)
- Exit codes

**TUI:**
- State management
- Widget rendering
- Keyboard event handling

### Integration Tests

**End-to-end workflows:**
- Discovery → Enrollment → Command execution
- Mock gRPC server for testing
- Certificate validation (valid, expired, mismatched)
- Timeout handling
- Network error recovery

**Platform-specific:**
- Test on Linux, macOS, Windows
- Verify file permissions (0600 for private keys)
- Platform-specific paths (`directories` crate)

### Manual Testing Scenarios

1. **Enrollment:**
   - QR code enrollment (copy/paste)
   - Approval pairing with verification code
   - Reject pairing
   - Timeout during pairing

2. **Command Execution:**
   - All parameter types (slider, text, toggle, dropdown)
   - Commands with/without parameters
   - Streaming output
   - Long-running commands
   - Command timeout
   - Network interruption

3. **Multi-server:**
   - Enroll to multiple servers
   - Switch between servers
   - Remove enrollment
   - Server offline/unreachable

4. **TUI Specific:**
   - Terminal resize handling
   - All keybindings
   - Search and filtering
   - Settings persistence

---

## Platform Support

### Target Platforms

| Platform | Tier | Notes |
|----------|------|-------|
| Linux x86_64 | 1 | Primary development platform |
| macOS x86_64 | 1 | Full support |
| macOS ARM64 | 1 | Full support (Apple Silicon) |
| Windows x86_64 | 2 | Best-effort, may have limitations |

**Tier 1:** Full support, tested on CI
**Tier 2:** Best-effort support, community-tested

### Cross-Compilation

Use `cross` for building on different platforms:

```bash
# Linux
cargo build --release

# macOS (from Linux)
cross build --release --target x86_64-apple-darwin

# Windows (from Linux)
cross build --release --target x86_64-pc-windows-gnu
```

### Platform-Specific Considerations

**Shell Commands:**
- Linux/macOS: `/bin/sh -c`
- Windows: `cmd.exe /C`
- Handled by server, transparent to client

**File Paths:**
- Use `directories` crate for cross-platform paths
- Normalize path separators

**Permissions:**
- Linux/macOS: `chmod 0600` for private keys
- Windows: Use ACLs (via `std::os::windows`)

---

## Comparison with Android Client

| Feature | Android | CLI | TUI |
|---------|---------|-----|-----|
| **Discovery** | NSD API | mdns-sd | mdns-sd |
| **Enrollment (QR)** | Camera scan | Copy/paste | Copy/paste |
| **Enrollment (Approval)** | ✅ | ✅ | ✅ |
| **Verification Code** | ✅ | ✅ | ✅ |
| **Certificate Storage** | Keystore (hardware) | Files (0600) | Files (0600) |
| **Certificate Pinning** | ✅ | ✅ | ✅ |
| **Command List** | ✅ | ✅ | ✅ |
| **All Parameter Types** | ✅ | ✅ | ✅ |
| **Streaming Output** | ✅ | ✅ | ✅ |
| **Config Hot-Reload** | ✅ | ✅ (Phase 3) | ✅ (Phase 3) |
| **Dynamic Defaults** | ✅ | ✅ (Phase 3) | ✅ (Phase 3) |
| **Relay Support** | ✅ | ✅ (Phase 3) | ✅ (Phase 3) |
| **Favorites** | ✅ | ❌ | ✅ (Phase 3) |
| **Settings UI** | ✅ | Config file | ✅ |
| **Background Running** | ✅ | ❌ | ❌ |
| **Notifications** | ✅ | ❌ | ❌ |
| **Scripting** | ❌ | ✅ | ❌ |

---

## Open Questions

1. **Certificate encryption:** Should we offer optional password encryption for private keys? Or is file permissions (0600) sufficient?

2. **Command history:** Should CLI maintain a history of executed commands? Location: `~/.config/handcontrol/history.toml`?

3. **Concurrent commands:** Should CLI support executing the same command on multiple servers in parallel?

4. **TUI persistence:** Should TUI remember last selected server and command across sessions?

5. **Logging:** Where should client logs go? `~/.local/state/handcontrol/client.log` or stderr only?

6. **Update mechanism:** Auto-update checking? Or rely on package managers?

7. **Shell completion:** Priority for bash/zsh/fish completion scripts?

8. **Config merging:** Support system-wide config (`/etc/handcontrol/client.toml`) merged with user config?

---

## Security Audit Checklist

- [ ] Private keys stored with 0600 permissions
- [ ] Certificate validation on every connection
- [ ] Server certificate pinning (TOFU)
- [ ] Verification code matches Android algorithm
- [ ] Parameter values validated before sending
- [ ] No secrets logged (codes, tokens, private keys)
- [ ] Config files don't contain sensitive data
- [ ] TLS 1.3 with strong cipher suites (via rustls)
- [ ] Timeout on all network operations
- [ ] Input sanitization for all user inputs

---

## Example Configurations

### Example 1: Home Lab Server

```toml
# ~/.config/handcontrol/client.toml

[discovery]
auto_discover = true
timeout_seconds = 10
prefer_ipv6 = false  # Home network is IPv4-only

[connection]
timeout_seconds = 5
command_timeout_seconds = 600  # Some scripts take a while

[device]
name = "Home Desktop"
model = "Custom Build"

[tui]
show_timestamps = true
color_scheme = "dark"
confirm_commands = true  # Always confirm before executing
```

### Example 2: Automation Server (CLI-only)

```toml
# ~/.config/handcontrol/client.toml

[discovery]
auto_discover = false  # Use explicit server IDs
timeout_seconds = 3

[connection]
timeout_seconds = 3
retry_attempts = 5
retry_delay_ms = 500

[cli]
output_format = "json"  # For parsing with jq
show_headers = false
color_output = "never"  # Clean output for scripts
```

---

## Appendix: Ratatui Widgets

### Core Widgets Used

**From ratatui:**
- `List`: Server list, command list
- `Table`: Server/command details
- `Paragraph`: Help text, status messages
- `Gauge`: Progress bars, sliders
- `Block`: Borders and titles
- `Tabs`: Multi-view navigation (future)

**Custom Widgets:**
- `ServerListWidget`: Server discovery results with status indicators
- `CommandBrowserWidget`: Searchable/filterable command list
- `ParameterFormWidget`: Dynamic form for command parameters
- `StreamingOutputWidget`: Scrollable, colorized output viewer
- `VerificationCodeWidget`: Large, centered verification code display

### Color Scheme

**Default theme:**
- Normal text: Default terminal color
- Selected: Cyan background
- Success: Green
- Error: Red
- Warning: Yellow
- Info: Blue
- Timestamp: Gray

**Supports terminal themes** (inherits from terminal colors)

---

## Summary

The HandControl CLI/TUI Client extends the HandControl ecosystem to terminal-based workflows, serving power users, automation scenarios, and users without mobile devices. By providing both a scriptable CLI and an interactive TUI, we cater to a wide range of use cases while maintaining security parity with the Android client.

**Key differentiators:**
- Two separate binaries optimized for their use cases
- Full feature parity with Android (enrollment, mTLS, all parameter types)
- Unix/shell integration (pipes, fzf, scripting)
- Keyboard-driven workflows (TUI)
- Cross-platform support (Linux, macOS, Windows)

**Next steps:**
1. Review and approve this PRD
2. Set up project structure (workspace with 3 crates)
3. Implement Phase 1 (basic CLI)
4. Gather user feedback
5. Implement Phase 2 (TUI)

---

**Document Status:** Ready for review
