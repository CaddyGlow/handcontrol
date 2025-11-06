# Android Interactive Terminal Integration Plan

## 0. Licensing Due Diligence
- HandControl is released under the MIT License (`LICENSE` in the repo root); all client artifacts must continue to comply with MIT distribution requirements.
- Termux’s `terminal-view` and `terminal-emulator` modules are Apache 2.0 exceptions. Vendor only those directories and keep GPL-only Termux code (e.g., `termux-shared`) out of the tree.
- Bundle the Apache 2.0 license text and attribution for the Termux-derived components alongside the app, and track future upstream syncs for new copyright holders.
- Keep `licenses/Apache-2.0-Termux-Terminal.txt` and `THIRD_PARTY_NOTICES.md` current whenever the vendored terminal code changes.

## 1. Vendor the Termux Terminal Widget
- Copy `terminal-view` (and required `terminal-emulator` sources) from the Termux repository into `android/terminal-view`.
- Add the module to `settings.gradle.kts` and add `implementation(project(":terminal-view"))` to the app module.
- Update the vendored module to build with AGP 8.7 / Kotlin 2.0; fix any namespace clashes or missing AndroidX dependencies.
- Run `./gradlew :terminal-view:assemble` to ensure the module compiles in isolation.

## 2. Remote Session Adapter
- Make `TerminalSession` extensible and implement `RemoteTerminalSession` extending it.
  - Accept a `ShellSession` (gRPC handle) instead of spawning a local process.
  - Override input APIs so `write(byte[])` forwards to `ShellSession.writeInput`.
  - Process PTY output by feeding `ShellSession.events` back into the terminal emulator (`processInput(data)`).
  - Forward resize notifications (`onTerminalSizeChanged`) to `ShellSession.resize`.
  - Surface lifecycle callbacks (`onSessionFinish`, exit codes, errors) so the UI can update connection state.

## 3. Compose ↔ Terminal Bridge
- Build a `TerminalSurface` composable that wraps `TerminalView` via `AndroidView`.
- Handle lifecycle: request focus on appearance, detach the session and close the gRPC stream inside `DisposableEffect`.
- Provide hooks to expose clipboard/paste and soft keyboard toggling through the Compose UI layer.

## 4. Shell Screen Workflow Updates
- Replace the existing text-based output with the real terminal view once the session is connecting/active.
- Keep the parameter form; once validated, construct `RemoteTerminalSession` and launch `start()` in `viewModelScope`.
- Expose session actions: `Start`, `Retry`, `Close`, plus status banner (connected, exit code, failure).

## 5. Server/Protocol Considerations
- Ensure the server’s shell capability emits raw PTY bytes (no newline munging) and keeps `binary=true`.
- Verify resize events are honoured end-to-end (already implemented in `shell_interactive.rs`).
- Optionally add heartbeat telemetry to detect stalled sessions on the client side.

## 6. Input Experience
- Rely on Termux’s built-in keyboard handling for hardware keys and modifiers (Ctrl/Alt).
- Add an optional Compose action row for soft shortcuts (Ctrl-C, Esc, Tab) that call `session.writeInput`.
- Confirm clipboard paste works; expose a UI affordance if necessary.

## 7. Testing & Validation
- Manual QA: run interactive programs (`vim`, `htop`, `less`), test resize, rotation, background/foreground, network drop.
- Add instrumentation tests using a fake `ShellSession` that replays canned escape sequences to verify rendering.
- Keep existing unit tests passing (`./gradlew :app:compileDebugKotlin`), and add smoke tests (`:app:connectedDebugAndroidTest`) once emulator automation is wired up.

## 8. Polish & Rollout
- Add loading/retry states and graceful close messaging to the UI.
- Document setup in the README (including licensing note and feature flag if used).
- Run lint/Detekt, ensure the vendored module obeys formatting (Spotless/Ktlint).
- After staging verification, enable the feature for all shell-interactive capabilities.
