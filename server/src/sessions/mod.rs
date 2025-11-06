pub mod manager;
pub mod state;

pub use state::{
    AttachmentState, BufferedOutput, OutputStream, SessionStateError, SessionStateSnapshot,
};

use tokio::sync::mpsc;

#[derive(Debug)]
pub struct SessionEndpoints {
    pub inbound: mpsc::Receiver<SessionClientEvent>,
    pub outbound: mpsc::Sender<SessionServerEvent>,
}

#[derive(Debug)]
pub enum SessionClientEvent {
    Input {
        data: bytes::Bytes,
        binary: bool,
    },
    Resize {
        cols: u32,
        rows: u32,
    },
    Heartbeat {
        timestamp_ms: i64,
    },
    Close {
        reason: Option<String>,
    },
    Resume {
        resume_token: String,
        last_stdout_sequence: Option<u64>,
        last_stderr_sequence: Option<u64>,
    },
}

#[derive(Debug, Clone)]
pub enum SessionServerEvent {
    Ready {
        message: Option<String>,
    },
    Output {
        data: bytes::Bytes,
        stderr: bool,
        binary: bool,
        timestamp_ms: Option<i64>,
    },
    Exit {
        exit_code: i32,
        timed_out: bool,
        message: Option<String>,
    },
    Error {
        message: String,
        code: Option<i32>,
    },
    HeartbeatAck {
        timestamp_ms: i64,
        latency_hint_ms: Option<i64>,
    },
    Closed {
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionId(uuid::Uuid);

impl SessionId {
    pub fn new() -> Self {
        SessionId(uuid::Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
