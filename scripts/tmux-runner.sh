#!/usr/bin/env bash
set -euo pipefail

# tmux-runner.sh - Run multiple commands in tmux panes with output capture
# Usage: ./tmux-runner.sh [--new] "command1" "command2" "command3" ...
#
# Each command runs in its own pane within a window named "runner" in the current tmux session.
# Output from each command is captured to /tmp/tmux-runner-<timestamp>-pane-<N>.log
# for LLM review.
#
# By default, reuses existing "runner" window if it exists.
# Use --new flag to force creation of a new window (kills existing one).
#
# Example:
#   ./tmux-runner.sh \
#     "RUST_LOG=trace cargo run --bin relay" \
#     "cargo test --workspace" \
#     "cargo build --release"
#
#   ./tmux-runner.sh --new "RUST_LOG=trace cargo run"

# Check if we're inside a tmux session
if [ -z "${TMUX:-}" ]; then
    echo "Error: This script must be run from within a tmux session"
    exit 1
fi

# Parse flags
FORCE_NEW=false
if [ $# -gt 0 ] && [ "$1" = "--new" ]; then
    FORCE_NEW=true
    shift
fi

# Check if at least one command was provided
if [ $# -eq 0 ]; then
    echo "Usage: $0 [--new] \"command1\" \"command2\" \"command3\" ..."
    echo "Example: $0 \"cargo build\" \"cargo test\" \"cargo check\""
    echo ""
    echo "Complex commands with env vars are supported:"
    echo "  $0 \"RUST_LOG=trace cargo run -- --config ~/.config/app.toml\""
    echo ""
    echo "Options:"
    echo "  --new    Force creation of new window (kills existing 'runner' window)"
    exit 1
fi

# Get current session name
SESSION=$(tmux display-message -p '#S')

# Check for existing runner window
EXISTING_WINDOW=$(tmux list-windows -t "$SESSION" -F '#{window_index} #{window_name}' 2>/dev/null | grep ' runner$' | cut -d' ' -f1 || true)

if [ -n "$EXISTING_WINDOW" ]; then
    if [ "$FORCE_NEW" = true ]; then
        echo "Killing existing runner window (${EXISTING_WINDOW}) and creating new one..."
        tmux kill-window -t "${SESSION}:${EXISTING_WINDOW}"
        EXISTING_WINDOW=""
    else
        echo "Reusing existing runner window (${EXISTING_WINDOW})"
        # Kill all panes except the first one
        PANE_COUNT=$(tmux list-panes -t "${SESSION}:${EXISTING_WINDOW}" | wc -l)
        if [ "$PANE_COUNT" -gt 1 ]; then
            # Kill panes from highest to lowest (except pane 1)
            for ((i=$PANE_COUNT; i>1; i--)); do
                tmux kill-pane -t "${SESSION}:${EXISTING_WINDOW}.${i}" 2>/dev/null || true
            done
        fi
        WINDOW_TARGET="${SESSION}:${EXISTING_WINDOW}"
        WINDOW_INDEX="$EXISTING_WINDOW"
    fi
fi

# Create output directory first (using timestamp to ensure uniqueness)
TIMESTAMP=$(date +%s)
OUTPUT_DIR="/tmp/tmux-runner-${TIMESTAMP}"
mkdir -p "$OUTPUT_DIR"

# Create a new window if needed
if [ -z "$EXISTING_WINDOW" ]; then
    WINDOW_TARGET=$(tmux new-window -P -F '#{session_name}:#{window_index}' -n "runner")
    WINDOW_INDEX=$(echo "$WINDOW_TARGET" | cut -d: -f2)
    echo "Created new runner window (${WINDOW_INDEX}) in session '${SESSION}'"
else
    echo "Window ${WINDOW_INDEX} in session '${SESSION}'"
fi

echo "Output files will be in: ${OUTPUT_DIR}/"

# Helper function to safely send commands to tmux
send_command_to_pane() {
    local pane=$1
    local cmd=$2
    local logfile=$3

    # Use printf %q to properly escape the command for shell execution
    local escaped_cmd
    escaped_cmd=$(printf '%q' "$cmd")

    # Create a wrapper script that will execute the command and capture output
    local wrapper="${logfile}.sh"
    cat > "$wrapper" <<EOF
#!/usr/bin/env bash
set +e
$cmd 2>&1 | tee "$logfile"
exit_code=\${PIPESTATUS[0]}
echo "" >> "$logfile"
echo "Command finished with exit code: \$exit_code" >> "$logfile"
exit \$exit_code
EOF
    chmod +x "$wrapper"

    # Send the wrapper script to the pane
    tmux send-keys -t "$pane" "bash '$wrapper'" C-m
}

# First command runs in the first pane (pane 1)
FIRST_CMD="$1"
LOGFILE="${OUTPUT_DIR}/pane-1.log"
shift

# Set up the first pane with logging
# Just use the window target - it defaults to the first pane
send_command_to_pane "${WINDOW_TARGET}" "$FIRST_CMD" "$LOGFILE"

echo "  Pane 1: $FIRST_CMD"
echo "          -> $LOGFILE"

# For remaining commands, split the window and run each in a new pane
PANE_INDEX=2
for cmd in "$@"; do
    LOGFILE="${OUTPUT_DIR}/pane-${PANE_INDEX}.log"

    # Split the window vertically (creates a new pane)
    tmux split-window -t "$WINDOW_TARGET" -v

    # Tile all panes evenly
    tmux select-layout -t "$WINDOW_TARGET" tiled

    # Send the command to the new pane
    send_command_to_pane "${WINDOW_TARGET}.${PANE_INDEX}" "$cmd" "$LOGFILE"

    echo "  Pane ${PANE_INDEX}: $cmd"
    echo "          -> $LOGFILE"

    ((PANE_INDEX++))
done

echo ""
echo "All commands started. Switch to window ${WINDOW_INDEX} to view progress."
echo "To read all output files: cat ${OUTPUT_DIR}/pane-*.log"
echo "To switch to the window: tmux select-window -t ${WINDOW_INDEX}"
