use bytes::Bytes;
use std::collections::VecDeque;
use std::time::SystemTime;

/// Identifies which logical output stream produced a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

impl OutputStream {
    pub fn from_stderr_flag(stderr: bool) -> Self {
        if stderr {
            OutputStream::Stderr
        } else {
            OutputStream::Stdout
        }
    }
}

/// Buffered output frame retained for replay.
#[derive(Debug, Clone)]
pub struct BufferedOutput {
    pub stream: OutputStream,
    pub sequence: u64,
    pub data: Bytes,
    pub binary: bool,
    pub timestamp_ms: Option<i64>,
}

/// Tracks the current attachment state for a session.
#[derive(Debug, Clone)]
pub struct AttachmentState {
    pub attachment_id: uuid::Uuid,
    pub client_fingerprint: Option<String>,
    pub attached_at: SystemTime,
    pub last_heartbeat: Option<SystemTime>,
}

/// Detailed snapshot of a session state for diagnostics or API responses.
#[derive(Debug, Clone)]
pub struct SessionStateSnapshot {
    pub resume_token: String,
    pub stdout_next_sequence: u64,
    pub stderr_next_sequence: u64,
    pub buffer_len: usize,
    pub attachment: Option<AttachmentState>,
    pub created_at: SystemTime,
    pub last_activity: SystemTime,
    pub last_detached_at: Option<SystemTime>,
}

/// Errors surfaced while mutating the session state.
#[derive(Debug)]
pub enum SessionStateError {
    AlreadyAttached,
    NotAttached,
}

impl std::fmt::Display for SessionStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStateError::AlreadyAttached => {
                write!(f, "session already has an active attachment")
            }
            SessionStateError::NotAttached => write!(f, "session has no active attachment"),
        }
    }
}

impl std::error::Error for SessionStateError {}

/// Mutable state scoped to a single capability session.
pub struct SessionState {
    resume_token: String,
    stdout_next_sequence: u64,
    stderr_next_sequence: u64,
    buffer_capacity: usize,
    output_buffer: VecDeque<BufferedOutput>,
    attachment: Option<AttachmentState>,
    created_at: SystemTime,
    last_activity: SystemTime,
    last_detached_at: Option<SystemTime>,
}

impl SessionState {
    pub fn new(buffer_capacity: usize) -> Self {
        let now = SystemTime::now();
        Self {
            resume_token: uuid::Uuid::new_v4().to_string(),
            stdout_next_sequence: 1,
            stderr_next_sequence: 1,
            buffer_capacity,
            output_buffer: VecDeque::with_capacity(buffer_capacity.min(16)),
            attachment: None,
            created_at: now,
            last_activity: now,
            last_detached_at: None,
        }
    }

    pub fn resume_token(&self) -> &str {
        &self.resume_token
    }

    pub fn rotate_resume_token(&mut self) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        self.resume_token = token.clone();
        token
    }

    pub fn validate_resume_token(&self, token: &str) -> bool {
        self.resume_token == token
    }

    pub fn snapshot(&self) -> SessionStateSnapshot {
        SessionStateSnapshot {
            resume_token: self.resume_token.clone(),
            stdout_next_sequence: self.stdout_next_sequence,
            stderr_next_sequence: self.stderr_next_sequence,
            buffer_len: self.output_buffer.len(),
            attachment: self.attachment.clone(),
            created_at: self.created_at,
            last_activity: self.last_activity,
            last_detached_at: self.last_detached_at,
        }
    }

    pub fn try_attach(
        &mut self,
        client_fingerprint: Option<String>,
    ) -> Result<AttachmentState, SessionStateError> {
        if self.attachment.is_some() {
            return Err(SessionStateError::AlreadyAttached);
        }

        let attachment = AttachmentState {
            attachment_id: uuid::Uuid::new_v4(),
            client_fingerprint,
            attached_at: SystemTime::now(),
            last_heartbeat: None,
        };

        self.last_activity = attachment.attached_at;
        self.attachment = Some(attachment.clone());
        Ok(attachment)
    }

    pub fn try_detach(&mut self) -> Result<AttachmentState, SessionStateError> {
        match self.attachment.take() {
            Some(attachment) => {
                self.last_activity = SystemTime::now();
                self.last_detached_at = Some(self.last_activity);
                Ok(attachment)
            }
            None => Err(SessionStateError::NotAttached),
        }
    }

    pub fn attachment(&self) -> Option<&AttachmentState> {
        self.attachment.as_ref()
    }

    pub fn touch_heartbeat(&mut self) -> Result<(), SessionStateError> {
        if let Some(attachment) = self.attachment.as_mut() {
            attachment.last_heartbeat = Some(SystemTime::now());
            self.last_activity = attachment.last_heartbeat.unwrap();
            Ok(())
        } else {
            Err(SessionStateError::NotAttached)
        }
    }

    pub fn record_output(
        &mut self,
        stream: OutputStream,
        data: Bytes,
        binary: bool,
        timestamp_ms: Option<i64>,
    ) -> BufferedOutput {
        let sequence = match stream {
            OutputStream::Stdout => {
                let seq = self.stdout_next_sequence;
                self.stdout_next_sequence += 1;
                seq
            }
            OutputStream::Stderr => {
                let seq = self.stderr_next_sequence;
                self.stderr_next_sequence += 1;
                seq
            }
        };

        let frame = BufferedOutput {
            stream,
            sequence,
            data: data.clone(),
            binary,
            timestamp_ms,
        };

        self.output_buffer.push_back(frame.clone());
        if self.output_buffer.len() > self.buffer_capacity {
            self.output_buffer.pop_front();
        }

        self.last_activity = SystemTime::now();

        frame
    }

    pub fn outputs_since(
        &self,
        last_stdout_sequence: Option<u64>,
        last_stderr_sequence: Option<u64>,
    ) -> Vec<BufferedOutput> {
        self.output_buffer
            .iter()
            .filter(|frame| match frame.stream {
                OutputStream::Stdout => last_stdout_sequence
                    .map(|seq| frame.sequence > seq)
                    .unwrap_or(true),
                OutputStream::Stderr => last_stderr_sequence
                    .map(|seq| frame.sequence > seq)
                    .unwrap_or(true),
            })
            .cloned()
            .collect()
    }

    pub fn stdout_next_sequence(&self) -> u64 {
        self.stdout_next_sequence
    }

    pub fn stderr_next_sequence(&self) -> u64 {
        self.stderr_next_sequence
    }
}
