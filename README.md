# HandControl

HandControl is a cross-platform remote command execution stack. It pairs a self-hosted Rust server with desktop and mobile clients so you can expose a curated set of shell commands through a mutually authenticated gRPC API. Enrollment supports both QR-code bootstrapping and approval flows, with certificate pinning on every client and optional relay tunnelling for machines that are not directly reachable.

## Components
- `server/` (`handcontrol-server`): gRPC service that executes vetted shell commands, manages enrollment tokens, stores authorized client certificates, streams command output, and advertises itself via mDNS.
- `client-lib/`: Shared Rust client SDK providing discovery, enrollment, TLS channel setup, command invocation, and local persistence of server trust roots.
- `cli/` (`handcontrol-cli`): Terminal client for discovery, enrollment, command execution, and configuration management.
- `tui/` (`handcontrol-tui`): Ratatui-based interface for browsing servers, commands, and streaming output.
- `relay/` (`handcontrol-relay`): Optional websocket-based rendezvous service that brokers tunnels between enrolled clients and servers when direct connectivity is unavailable.
- `android/`: Jetpack Compose client that mirrors the Rust feature set (see `android/README.md` for build details).
- `proto/handcontrol.proto`: Shared protobuf definitions used by every component.

## Getting Started
```bash
# Prerequisites: Rust 1.80+ (for the 2024 edition workspace), protoc, and openssl headers.

# Build everything
cargo build --workspace

# Start the server (generates default config at first launch)
cargo run -p handcontrol-server -- serve

# Generate a QR payload in the terminal for quick enrollment
cargo run -p handcontrol-server -- enroll --qr
```

The server writes its configuration to `~/.config/handcontrol/config.toml` (or the platform equivalent) on first run. Edit this file to curate command definitions, adjust enrollment policies, or enable the relay client. Set `HANDCONTROL_CONFIG_DIR=/path/to/dir` before launching any HandControl binary to override the entire suite's config root (server, CLI, TUI, and shared client storage).

## Clients
- **CLI:** `cargo run -p handcontrol-cli -- discover` for mDNS discovery, `... -- enroll qr --payload '<json>'` for QR onboarding, and `... -- exec <server> <command_id>` to invoke commands.
- **TUI:** `cargo run -p handcontrol-tui` for an interactive dashboard that handles discovery, enrollment, command execution, and streaming output.
- **Android:** Follow `android/README.md` to build the Compose client. It shares the same gRPC surface and certificate format as the Rust tooling.

## Relay Service
To operate across NATs or different networks, provision the relay:
```bash
# Edit relay.toml to define bind address, TLS material, and per-server secrets
cargo run -p handcontrol-relay -- --config relay.toml
```
The server can be configured to auto-connect, register its public key, and mint JWT relay tokens for enrolled clients.

Relay also respects `HANDCONTROL_RELAY_CONFIG_DIR` (preferred) or `HANDCONTROL_CONFIG_DIR` to locate `relay.toml` automatically and to store generated credentials.

## Development Notes
- Protobuf code is generated automatically during builds via `tonic-prost`; rerun `cargo build` after modifying `proto/handcontrol.proto`.
- `docs/` contains the PRD, security model, and feature design notes. Start with `docs/PROJECT_STRUCTURE.md` and `docs/SECURITY.md`.
- A Nix flake (`flake.nix`) is provided for reproducible toolchains and cross-compilation targets.
- Unit tests are available across the workspace: `cargo test --workspace`.

### Local Test Harness
Run `scripts/setup-test-env.sh` to spin up a self-contained environment that builds the binaries, launches a relay + server with aligned configs, enrolls the CLI via QR payload, and captures artifacts under `.handcontrol-test-env/` (configurable with `--dir`). Use `--keep-running` to leave the services alive for manual experimentation.

## Known Gaps
- Documentation predates portions of the workspace split. Double-check crate names and paths against the current tree when following older guides.

## License
HandControl is released under the MIT License; see `LICENSE` for details. Third-party modules, such as the Termux-derived terminal components, remain under their original open-source terms and are documented in `THIRD_PARTY_NOTICES.md` and `docs/android-terminal-integration-plan.md`.
