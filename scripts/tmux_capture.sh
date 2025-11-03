#!/usr/bin/env bash
#
# Launch one or more tmux panes that run the commands provided on the command
# line. Each pane logs its output to a run-specific directory so the capture can
# be reused later.
#
# Usage:
#   scripts/capture_relay_trace.sh "name::command" ["other-name::other command"]
#   scripts/capture_relay_trace.sh "some command without a name"
#
# Names are optional. If omitted, one is generated (`command1`, `command2`, ...).

set -euo pipefail

if [[ $# -eq 0 ]]; then
  printf 'Usage: %s "name::command" ["other-name::..." ...]\n' "$0" >&2
  exit 1
fi

if ! command -v tmux >/dev/null 2>&1; then
  echo "Error: tmux not found in PATH. Install tmux to use this script." >&2
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
RUN_DIR="$(mktemp -d -t handcontrol-trace-${TIMESTAMP}-XXXXXX)"

declare -a CMD_NAMES=()
declare -a CMD_CMDS=()
declare -a CMD_LOGS=()
declare -a CMD_RUNNERS=()

slugify() {
  local value="$1"
  value="$(printf '%s' "${value}" | tr '[:upper:]' '[:lower:]')"
  value="$(printf '%s' "${value}" | tr -cs 'a-z0-9_-.' '_')"
  if [[ -z "${value}" ]]; then
    value="command"
  fi
  echo "${value}"
}

idx=0
for spec in "$@"; do
  ((idx+=1))
  name="command${idx}"
  command="${spec}"
  if [[ "${spec}" == *"::"* ]]; then
    name_candidate="${spec%%::*}"
    command="${spec#*::}"
    if [[ -n "${name_candidate}" ]]; then
      name="${name_candidate}"
    fi
  fi
  slug="$(slugify "${name}")"
  log_path="${RUN_DIR}/${slug}.log"
  CMD_NAMES+=("${name}")
  CMD_CMDS+=("${command}")
  CMD_LOGS+=("${log_path}")
done

SESSION_NAME="capture-${TIMESTAMP}-$$"
WINDOW_NAME="run"

tmux new-session -d -s "${SESSION_NAME}" -n "${WINDOW_NAME}" -c "${ROOT_DIR}"
WINDOW_TARGET="${SESSION_NAME}:${WINDOW_NAME}"

tmux set-window-option -t "${WINDOW_TARGET}" remain-on-exit on >/dev/null

if ! base_pane="$(tmux display-message -p -t "${WINDOW_TARGET}.0" '#{pane_id}')" 2>/dev/null; then
  echo "Error: unable to determine tmux pane for ${WINDOW_TARGET}." >&2
  exit 1
fi

cleanup() {
  for runner in "${CMD_RUNNERS[@]}"; do
    [[ -n "${runner}" && -f "${runner}" ]] && rm -f "${runner}"
  done
  echo
  echo "Logs stored under: ${RUN_DIR}"
}
trap cleanup EXIT

send_command() {
  local pane="$1"
  local command="$2"
  local log="$3"
  local runner
  runner="$(mktemp "${RUN_DIR}/runner-XXXX.sh")"
  CMD_RUNNERS+=("${runner}")
  cat >"${runner}" <<EOF
#!/usr/bin/env bash
set -euo pipefail
cd "${ROOT_DIR}"
${command} 2>&1 | tee -a "${log}"
EOF
  chmod +x "${runner}"
  tmux send-keys -t "${pane}" "${runner}" C-m
}

split_target="${base_pane}"
for i in "${!CMD_CMDS[@]}"; do
  if [[ "${i}" -gt 0 ]]; then
    if ! split_target="$(tmux split-window -v -t "${split_target}" -c "${ROOT_DIR}" -P -F '#{pane_id}')" 2>/dev/null; then
      echo "Error: unable to create additional pane." >&2
      exit 1
    fi
  fi
  send_command "${split_target}" "${CMD_CMDS[$i]}" "${CMD_LOGS[$i]}"
done

tmux select-layout -t "${WINDOW_TARGET}" tiled >/dev/null 2>&1 || true
tmux select-pane -t "${base_pane}"

echo "Run directory: ${RUN_DIR}"
for i in "${!CMD_NAMES[@]}"; do
  echo "  ${CMD_NAMES[$i]} -> ${CMD_LOGS[$i]}"
done
echo

if [[ -n "${TMUX:-}" ]]; then
  tmux display-message -t "${SESSION_NAME}" "capture window ${WINDOW_NAME} ready"
  echo "Attach with: tmux attach -t ${SESSION_NAME}"
else
  tmux attach -t "${SESSION_NAME}" || true
fi
