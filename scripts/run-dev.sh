#!/usr/bin/env bash
project_root="$(dirname "$(dirname "$(realpath "$0")")")"

bash ${project_root}/scripts/tmux-runner.sh \
  "RUST_LOG=handcontrol_relay=trace cargo run --bin handcontrol-relay -- --config ~/.config/handcontrol/relay.toml" \
  "adb logcat --pid=\$(adb shell pidof -s com.handcontrol)" \
  "RUST_LOG=handcontrol=trace,hyper=trace,tower=debug cargo run -p handcontrol-server --bin handcontrol"
