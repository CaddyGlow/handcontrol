# Resumable Sessions

This document explains how the resumable session flow works in HandControl, what the server guarantees, and how clients—including the CLI—should interact with the new protocol.

## Overview

- Every realtime capability session now exposes a resume token alongside the traditional session identifier.
- Servers keep PTY/process state alive when the transport drops, buffer recent stdout/stderr output, and allow the same enrolled user to reconnect.
- Clients reconnect by sending `SessionResume` as their first message on the `OpenSession` RPC, providing the last output sequence numbers they observed.
- Resume support is single-user only—tokens are scoped to the original attachment and are rotated after each successful resume.

## Server Behaviour

### Session Tokens & Attachment Management

- The session manager assigns a random `resume_token` during session creation and rotates that token whenever a client successfully resumes.
- Only one attachment is allowed at a time. Detaching (e.g., network failure) keeps the capability running but releases stdin focus.
- Each `SessionReady` message now includes the current resume token so clients can persist it locally.

### Buffered Output & Sequence Numbers

- The server tracks stdout and stderr streams independently, recording monotonically-increasing sequence numbers.
- The last ~1024 output frames are retained (configurable via the session manager); this window is replayed during resume according to client-provided cursors.
- During resume the server delivers buffered output first, then emits a `SessionResumeAck` with the refreshed token to confirm that live streaming has resumed.

### Error Handling

- Resumes for unknown session IDs return `NOT_FOUND`.
- Invalid or stale resume tokens return `PERMISSION_DENIED`.
- Concurrent resume attempts are rejected with `RESOURCE_EXHAUSTED`.

## Client Responsibilities

### Library Integrations

- `CapabilitySession` now exposes `resume_token()`, `last_stdout_sequence()`, `last_stderr_sequence()`, and `resume_state()` helper methods.
- Use `resume_capability_session()` to reconnect using a saved `CapabilitySessionResumeState`.
- Always send the last seen stdout/stderr sequence IDs when resuming so the server can replay only missing output.
- Handle `SessionResumeAck` messages by updating the stored resume token before persisting state.

### CLI

- The CLI automatically retries a dropped realtime session using the saved resume state. Users will see a brief status message indicating that the session is being resumed.
- Resume attempts preserve terminal size (the CLI replays a resize event when reconnecting) and continue streaming output seamlessly after buffered replay completes.
- Explicit exits (`Ctrl+C` followed by prompt confirmation, or `exit` inside the shell) still close the session permanently.

### Other Clients

- Android/TUI clients should persist the resume token and last sequences locally (e.g., in ViewModel state) and initiate resume flows on reconnection.
- When a client cannot resume—because the session expired or buffers were pruned—surface clear UI messaging and allow the user to start a new session.

## gRPC Protocol Changes

| Message | Field | Purpose |
| --- | --- | --- |
| `SessionReady` | `resume_token` | Token required for future resume attempts |
| `SessionOutput` | `sequence` | Monotonic per-stream sequence number for replay alignment |
| `SessionClientMessage` | `SessionResume` | First message when resuming; carries `resume_token`, `last_output_sequence`, and `last_error_sequence` |
| `SessionServerMessage` | `SessionResumeAck` | Confirms resume success and provides the next `resume_token` |

### Resume Flow Summary

1. Client stores `(session_id, resume_token, stdout_seq, stderr_seq)` after each message.
2. On reconnect it opens a new `OpenSession` stream and immediately sends:
   ```protobuf
   SessionClientMessage {
     session_id: "...",
     resume { resume_token: "...", last_output_sequence: 42, last_error_sequence: 10 }
   }
   ```
3. Server validates the token, replays missing output, then sends a `SessionResumeAck`.
4. Client updates its stored resume token and continues processing live output/events.

## Limitations & Future Work

- Resume is limited to the originating user/device; there is no delegation or shared viewing (see plan doc for future extensions).
- Session buffers are in-memory; a server restart clears active sessions.
- Snapshotting or migration of PTY state remains out-of-scope for the current implementation.
- Capability authors should ensure their runtime tolerates temporary pauses of stdin when no attachment is present.

Refer to `docs/resumable-session-plan.md` for the engineering roadmap and background context.
