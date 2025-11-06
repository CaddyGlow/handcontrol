# Resumable Session Implementation Plan

## Step 1 – Protocol Handshake Update
- Extend `proto/handcontrol.proto:206` with:
  - `SessionResumeRequest` and `SessionResumeAck` messages.
  - Sequence number fields on `SessionOutput`.
  - A `resume_token` field in `SessionReady`.
- Regenerate the gRPC bindings so `client-lib` and `server/src/grpc/proto.rs` reflect the new schema.
- Update documentation that describes the single-user resume flow and its message semantics.

## Step 2 – Session Manager Refactor
- Enhance `server/src/sessions/manager.rs:1` to track:
  - Active attachment metadata.
  - Resume token generation and validation.
  - Last delivered stdout/stderr sequence numbers.
  - An in-memory ring buffer for recent `SessionServerEvent::Output` entries.
- Abstract these responsibilities behind reusable traits/structs so other session-enabled features can reuse lifecycle, buffering, and token logic.
- Expose APIs for attach, detach, resume replay, and termination without forcing capability shutdown.

## Step 3 – gRPC Bridge Logic
- Modify `server/src/grpc/server.rs:860` to differentiate new opens from resume attempts.
- Validate resume tokens via the session manager, reject concurrent attachments, and send buffered output plus `SessionResumeAck` on success.
- Update stream handling so transport drops trigger a detach rather than a capability close.
- Adjust `forward_client_events` to emit explicit detach events instead of hard closing the session.

## Step 4 – Capability Shell Updates
- In `server/src/capabilities/shell_interactive.rs:70`, keep the PTY process alive when detach occurs.
- Pause stdin writes while no attachment is present but continue routing PTY output into the session buffer.
- Add lightweight sequence counters to emitted `SessionServerEvent::Output` messages.
- Ensure explicit shutdown requests still terminate the PTY and clear the session.

## Step 5 – Client Coordination
- Update `client-lib/src/commands.rs:508` and dependent CLI/TUI layers to:
  - Persist the `resume_token` and last stdout/stderr sequence numbers.
  - Send `SessionResumeRequest` on reconnect and handle `SessionResumeAck`.
  - Only emit `SessionClose` when the user explicitly ends the session.
  - Surface resume status/errors and handle buffered replay before live streaming resumes.
- Design the client-side resume helpers in a capability-agnostic way so other realtime features can adopt them.

## Step 6 – Validation & Documentation
- Extend server integration tests and CLI smoke tests to cover detach/reconnect flows.
- Document the resumable session behavior in user-facing guides (e.g., under `docs/`).
- Confirm Android/TUI clients either implement the resume protocol or gracefully fall back to fresh sessions.
- Capture reusable patterns (e.g., session lifecycle interfaces, buffering strategies) in developer docs to guide future session-enabled features.
