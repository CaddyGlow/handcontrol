#!/usr/bin/env bash
set -euo pipefail

# dev-runner.sh - Launch the relay, Android logcat, server, and an interactive shell
# in dedicated tmux panes. Logs for each pane are always written to the same
# directory so they can be tailed independently of the current tmux session.

if [ -z "${TMUX:-}" ]; then
    echo "Error: dev-runner.sh must be run inside a tmux session" >&2
    exit 1
fi

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "${SCRIPT_DIR}/.." && pwd)
REPO_ROOT_ESCAPED=$(printf '%q' "$REPO_ROOT")

WINDOW_NAME="handcontrol-dev"

STATE_ROOT="${XDG_STATE_HOME:-$HOME/.local/state}"
LOG_DIR="${STATE_ROOT}/handcontrol/dev-runner"

mkdir -p "$LOG_DIR"

declare -a COMMAND_LABELS=("relay" "android" "server" "shell")
declare -a COMMANDS=(
    "RUST_LOG=handcontrol_relay=trace cargo run --bin handcontrol-relay -- --config ~/.config/handcontrol/relay.toml"
    "adb logcat --pid=\$(adb shell pidof -s com.handcontrol)"
    "RUST_LOG=handcontrol=trace,hyper=trace,tower=debug cargo run -p handcontrol-server --bin handcontrol"
    ""
)

SESSION=$(tmux display-message -p '#S')
EXISTING_WINDOW=$(tmux list-windows -t "$SESSION" -F '#{window_index} #{window_name}' 2>/dev/null | awk -v name="$WINDOW_NAME" '$2 == name {print $1}' || true)

if [ -n "$EXISTING_WINDOW" ]; then
    echo "Replacing existing ${WINDOW_NAME} window (${EXISTING_WINDOW})"
    tmux kill-window -t "${SESSION}:${EXISTING_WINDOW}"
fi

WINDOW_TARGET=$(tmux new-window -P -F '#{session_name}:#{window_index}' -n "$WINDOW_NAME")
WINDOW_INDEX=$(echo "$WINDOW_TARGET" | cut -d: -f2)

echo "Logs for this run:"
for label in "${COMMAND_LABELS[@]}"; do
    echo "  ${LOG_DIR}/pane-${label}.log"
done

send_command_to_pane() {
    local pane="$1"
    local cmd="$2"
    local logfile="$3"

    : > "$logfile"

    # Reset any existing pipe and attach the pane output to the logfile.
    tmux pipe-pane -t "$pane" 2>/dev/null || true
    local pipe_cmd
    pipe_cmd=$(printf 'cat > %q' "$logfile")
    tmux pipe-pane -t "$pane" "$pipe_cmd"

    tmux send-keys -t "$pane" "cd ${REPO_ROOT_ESCAPED}" C-m

    if [ -n "$cmd" ]; then
        tmux send-keys -t "$pane" "$cmd; printf 'Command finished with exit code: %s\n' \$?" C-m
    fi
}

pane_ids=()
pane_ids+=("$(tmux display-message -t "$WINDOW_TARGET" -p '#{pane_id}')")

for idx in "${!COMMANDS[@]}"; do
    label="${COMMAND_LABELS[$idx]}"
    cmd="${COMMANDS[$idx]}"
    logfile="${LOG_DIR}/pane-${label}.log"

    if [ "$idx" -eq 0 ]; then
        target="${pane_ids[0]}"
    else
        target=$(tmux split-window -t "$WINDOW_TARGET" -v -P -F '#{pane_id}')
        tmux select-layout -t "$WINDOW_TARGET" tiled
        pane_ids+=("$target")
    fi

    send_command_to_pane "$target" "$cmd" "$logfile"

    if [ -n "$cmd" ]; then
        echo "Started pane ${label}: $cmd"
    else
        echo "Pane ${label}: interactive shell ready (logs in ${logfile})"
    fi
done

echo "tmux window ${WINDOW_NAME} ready in session ${SESSION} (index ${WINDOW_INDEX})."
