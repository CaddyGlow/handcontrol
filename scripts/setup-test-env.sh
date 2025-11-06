#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: setup-test-env.sh [options]

Creates a standalone HandControl test environment with coordinated server,
client, and relay configuration. The script generates config files, launches
the relay and server, enrolls the CLI via QR payload, and stores artifacts
under the selected directory.

Options:
  -d, --dir PATH        Output directory for generated configs/logs
                        (default: <repo>/.handcontrol-test-env)
      --server-port N   gRPC port for the test server (default: 50051)
      --relay-port N    Port for the local relay (default: 8443)
      --device-name STR Device name presented during enrollment
                        (default: "Test CLI Device")
      --keep-running    Leave relay and server running after enrollment
      --force           Remove any existing output directory before setup
  -h, --help            Show this help message
EOF
}

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "${SCRIPT_DIR}/.." && pwd)

BASE_DIR=""
SERVER_PORT="50051"
RELAY_PORT="8443"
DEVICE_NAME="Test CLI Device"
KEEP_RUNNING=false
FORCE=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    -d|--dir)
      [[ $# -lt 2 ]] && { echo "Missing value for $1" >&2; exit 1; }
      BASE_DIR="$2"
      shift 2
      ;;
    --server-port)
      [[ $# -lt 2 ]] && { echo "Missing value for $1" >&2; exit 1; }
      SERVER_PORT="$2"
      shift 2
      ;;
    --relay-port)
      [[ $# -lt 2 ]] && { echo "Missing value for $1" >&2; exit 1; }
      RELAY_PORT="$2"
      shift 2
      ;;
    --device-name)
      [[ $# -lt 2 ]] && { echo "Missing value for $1" >&2; exit 1; }
      DEVICE_NAME="$2"
      shift 2
      ;;
    --keep-running)
      KEEP_RUNNING=true
      shift
      ;;
    --force)
      FORCE=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage
      exit 1
      ;;
  esac
done

if [[ -z "$BASE_DIR" ]]; then
  BASE_DIR="${REPO_ROOT}/.handcontrol-test-env"
fi

if [[ "$SERVER_PORT" =~ ^[0-9]+$ ]] && (( SERVER_PORT > 0 && SERVER_PORT < 65536 )); then
  :
else
  echo "Invalid --server-port value: ${SERVER_PORT}" >&2
  exit 1
fi

if [[ "$RELAY_PORT" =~ ^[0-9]+$ ]] && (( RELAY_PORT > 0 && RELAY_PORT < 65536 )); then
  :
else
  echo "Invalid --relay-port value: ${RELAY_PORT}" >&2
  exit 1
fi

if [[ -d "$BASE_DIR" ]]; then
  if [[ "$FORCE" == true ]]; then
    rm -rf "$BASE_DIR"
  else
    if find "$BASE_DIR" -mindepth 1 -print -quit 2>/dev/null | grep -q .; then
      echo "Output directory $BASE_DIR already exists – use --force to replace it." >&2
      exit 1
    fi
  fi
fi

mkdir -p "$BASE_DIR"

CONFIG_DIR="${BASE_DIR}/config"
LOG_DIR="${BASE_DIR}/logs"
PAYLOAD_FILE="${BASE_DIR}/enrollment-payload.json"
RELAY_CONFIG="${BASE_DIR}/relay.toml"
mkdir -p "$CONFIG_DIR" "$LOG_DIR"

generate_uuid() {
  if command -v uuidgen >/dev/null 2>&1; then
    uuidgen | tr '[:upper:]' '[:lower:]'
  elif command -v python3 >/dev/null 2>&1; then
    python3 - <<'PY'
import uuid
print(uuid.uuid4())
PY
  elif command -v python >/dev/null 2>&1; then
    python - <<'PY'
import uuid
print(uuid.uuid4())
PY
  else
    echo "uuidgen or Python is required to generate a server ID" >&2
    exit 1
  fi
}

generate_secret() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -base64 32 | tr -d '\n'
  elif command -v python3 >/dev/null 2>&1; then
    python3 - <<'PY'
import os, base64
print(base64.b64encode(os.urandom(32)).decode(), end="")
PY
  elif command -v python >/dev/null 2>&1; then
    python - <<'PY'
import os, base64
print(base64.b64encode(os.urandom(32)).decode(), end="")
PY
  else
    head -c 32 /dev/urandom | base64 | tr -d '\n'
  fi
}

SERVER_ID=$(generate_uuid)
RELAY_SECRET=$(generate_secret)

echo "$SERVER_ID" > "${CONFIG_DIR}/server_id.txt"

cat > "${CONFIG_DIR}/config.toml" <<EOF
[server]
port = ${SERVER_PORT}
bind_address = "127.0.0.1"
mdns_service_name = "handcontrol-test"
mdns_instance_name = "HandControl Test Rig"

[security]
enrollment_token_ttl = 300
require_client_cert = false

[security.enrollment]
qr_code_enabled = true
approval_enabled = true
approval_timeout_seconds = 60
approval_notification = false

[network]
prefer_ipv6 = false
include_link_local = false
include_ula = false
prefer_stable_addresses = true
excluded_interface_prefixes = []
max_advertised_addresses = 2

[relay]
enabled = true
relay_server_url = "ws://127.0.0.1:${RELAY_PORT}"
relay_auth_secret = "${RELAY_SECRET}"
include_in_enrollment = true
auto_connect = true
max_relay_tunnels = 4
reconnect_delay_seconds = 5
relay_token_ttl_hours = 1
allow_self_signed_tls = true
debug_mode = true
websocket_subprotocol = "handcontrol-relay.v1"

[[capabilities]]
id = "echo.message"
name = "Echo Message"
description = "Echo a short string back to the caller"
tags = ["test"]
requires_confirmation = false
privileged = false
kind = "shell_script"
command = "echo {message}"
timeout_seconds = 5
show_output = true
session_mode = "one_shot"

[[capabilities.parameters]]
name = "message"
type = "text"
description = "Message to echo"
default = "Hello from HandControl"
options = []

[capabilities.acl]
allow = []
deny = []

[capabilities.env]
EOF

cat > "${CONFIG_DIR}/client.toml" <<EOF
[cli]
output_format = "json"
show_headers = false
color_output = "auto"
jiggle_resize_on_resume = true

[device]
name = "${DEVICE_NAME}"
model = "handcontrol-cli"
EOF

cat > "${RELAY_CONFIG}" <<EOF
bind_address = "127.0.0.1"
port = ${RELAY_PORT}
handshake_timeout_seconds = 5

[registration_secrets]
"${SERVER_ID}" = "${RELAY_SECRET}"
EOF

export HANDCONTROL_CONFIG_DIR="${CONFIG_DIR}"

echo "Building HandControl binaries..."
cargo build -p handcontrol-server -p handcontrol-cli -p handcontrol-relay >/dev/null

SERVER_BIN="${REPO_ROOT}/target/debug/handcontrol"
CLI_BIN="${REPO_ROOT}/target/debug/handcontrol-cli"
RELAY_BIN="${REPO_ROOT}/target/debug/handcontrol-relay"

for bin in "$SERVER_BIN" "$CLI_BIN" "$RELAY_BIN"; do
  if [[ ! -x "$bin" ]]; then
    echo "Expected binary not found or not executable: $bin" >&2
    exit 1
  fi
done

relay_pid=""
server_pid=""

cleanup() {
  if [[ -n "${server_pid}" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
    server_pid=""
  fi
  if [[ -n "${relay_pid}" ]]; then
    kill "$relay_pid" 2>/dev/null || true
    wait "$relay_pid" 2>/dev/null || true
    relay_pid=""
  fi
}

trap cleanup EXIT

wait_for_port() {
  local host=$1
  local port=$2
  local timeout=${3:-30}
  local start
  start=$(date +%s)

  while true; do
    if { exec 3<>"/dev/tcp/${host}/${port}"; } >/dev/null 2>&1; then
      exec 3>&- 3<&-
      return 0
    fi

    if (( $(date +%s) - start >= timeout )); then
      return 1
    fi

    sleep 0.25
  done
}

port_available() {
  local host=$1
  local port=$2
  if { exec 3<>"/dev/tcp/${host}/${port}"; } >/dev/null 2>&1; then
    exec 3>&- 3<&-
    return 1
  fi
  return 0
}

if ! port_available "127.0.0.1" "${RELAY_PORT}"; then
  echo "Relay port 127.0.0.1:${RELAY_PORT} is already in use. Choose another with --relay-port." >&2
  exit 1
fi

echo "Starting relay on port ${RELAY_PORT}..."
"$RELAY_BIN" --config "${RELAY_CONFIG}" >"${LOG_DIR}/relay.log" 2>&1 &
relay_pid=$!
if ! wait_for_port "127.0.0.1" "${RELAY_PORT}" 15; then
  echo "Relay failed to bind to 127.0.0.1:${RELAY_PORT}. Check ${LOG_DIR}/relay.log." >&2
  exit 1
fi
if ! kill -0 "${relay_pid}" 2>/dev/null; then
  echo "Relay process exited unexpectedly. Check ${LOG_DIR}/relay.log." >&2
  exit 1
fi

if ! port_available "127.0.0.1" "${SERVER_PORT}"; then
  echo "Server port 127.0.0.1:${SERVER_PORT} is already in use. Choose another with --server-port." >&2
  exit 1
fi

echo "Starting server on port ${SERVER_PORT}..."
RUST_LOG=${RUST_LOG:-info} "$SERVER_BIN" serve >"${LOG_DIR}/server.log" 2>&1 &
server_pid=$!
if ! wait_for_port "127.0.0.1" "${SERVER_PORT}" 20; then
  echo "Server failed to bind to 127.0.0.1:${SERVER_PORT}. Check ${LOG_DIR}/server.log." >&2
  exit 1
fi
if ! kill -0 "${server_pid}" 2>/dev/null; then
  echo "Server process exited unexpectedly. Check ${LOG_DIR}/server.log." >&2
  exit 1
fi

cert_timeout=20
while [[ ! -f "${CONFIG_DIR}/server.crt" ]]; do
  if (( cert_timeout <= 0 )); then
    echo "Server certificate did not appear at ${CONFIG_DIR}/server.crt. Check ${LOG_DIR}/server.log." >&2
    exit 1
  fi
  sleep 1
  ((cert_timeout--))
done

echo "Generating enrollment QR payload..."
if ! "$SERVER_BIN" enroll --qr --json > "${PAYLOAD_FILE}"; then
  echo "Failed to generate enrollment payload. See ${LOG_DIR}/server.log for details." >&2
  exit 1
fi

echo "Enrolling CLI using generated payload..."
if ! "$CLI_BIN" enroll qr --payload-file "${PAYLOAD_FILE}" --device-name "${DEVICE_NAME}" \
    >"${LOG_DIR}/cli-enroll.log" 2>&1; then
  echo "CLI enrollment failed. See ${LOG_DIR}/cli-enroll.log for output." >&2
  exit 1
fi

echo "Listing enrolled servers..."
if ! "$CLI_BIN" list-servers --json > "${LOG_DIR}/cli-list-servers.json"; then
  echo "Failed to list servers after enrollment. See ${LOG_DIR}/cli-list-servers.json." >&2
  exit 1
fi

if [[ "$KEEP_RUNNING" == true ]]; then
  trap - EXIT
  echo
  echo "Environment ready."
  echo "- Config root: ${CONFIG_DIR}"
  echo "- Relay config: ${RELAY_CONFIG}"
  echo "- Enrollment payload: ${PAYLOAD_FILE}"
  echo "- Relay log: ${LOG_DIR}/relay.log (PID ${relay_pid})"
  echo "- Server log: ${LOG_DIR}/server.log (PID ${server_pid})"
  echo "- CLI enrollment log: ${LOG_DIR}/cli-enroll.log"
  echo "- CLI list servers output: ${LOG_DIR}/cli-list-servers.json"
  echo
  echo "Relay and server are still running. Stop them with:"
  echo "  kill ${server_pid} ${relay_pid}"
else
  cleanup
  trap - EXIT
  echo
  echo "Environment staged under ${BASE_DIR}."
  echo "- Config root: ${CONFIG_DIR}"
  echo "- Relay config: ${RELAY_CONFIG}"
  echo "- Enrollment payload: ${PAYLOAD_FILE}"
  echo "- Relay log: ${LOG_DIR}/relay.log"
  echo "- Server log: ${LOG_DIR}/server.log"
  echo "- CLI enrollment log: ${LOG_DIR}/cli-enroll.log"
  echo "- CLI list servers output: ${LOG_DIR}/cli-list-servers.json"
  echo
  echo "All background services were stopped. Re-run the binaries manually if needed:"
  echo "  HANDCONTROL_CONFIG_DIR=\"${CONFIG_DIR}\" ${REPO_ROOT}/target/debug/handcontrol serve"
  echo "  ${REPO_ROOT}/target/debug/handcontrol-relay --config \"${RELAY_CONFIG}\""
fi
