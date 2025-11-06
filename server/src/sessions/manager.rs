use super::{
    SessionClientEvent, SessionEndpoints, SessionId, SessionServerEvent,
    state::{
        AttachmentState, BufferedOutput, OutputStream, SessionState, SessionStateError,
        SessionStateSnapshot,
    },
};
use crate::capabilities::{Capability, CapabilityMetadata, CapabilityOpenContext, SessionMode};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, warn};

const DEFAULT_OUTPUT_BUFFER_CAPACITY: usize = 1024;
const DEFAULT_EVENT_CHANNEL_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct SessionManager {
    inner: Arc<SessionManagerInner>,
}

struct SessionManagerInner {
    sessions: RwLock<HashMap<uuid::Uuid, Arc<SessionEntry>>>,
}

#[derive(Clone)]
pub struct SessionBroadcastEvent {
    pub event: SessionServerEvent,
    pub buffered_output: Option<BufferedOutput>,
    pub resume_token: Option<String>,
}

struct SessionEntry {
    capability_id: String,
    metadata: CapabilityMetadata,
    session_mode: SessionMode,
    client_sender: mpsc::Sender<SessionClientEvent>,
    _capability: Arc<dyn Capability>,
    state: Mutex<SessionState>,
    event_tx: broadcast::Sender<SessionBroadcastEvent>,
}

#[derive(Debug)]
pub enum SessionManagerError {
    NotFound,
    AlreadyAttached,
    NotAttached,
    InvalidResumeToken,
}

impl std::fmt::Display for SessionManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionManagerError::NotFound => write!(f, "session not found"),
            SessionManagerError::AlreadyAttached => {
                write!(f, "session already has an active attachment")
            }
            SessionManagerError::NotAttached => write!(f, "session has no active attachment"),
            SessionManagerError::InvalidResumeToken => write!(f, "invalid resume token"),
        }
    }
}

impl std::error::Error for SessionManagerError {}

impl From<SessionStateError> for SessionManagerError {
    fn from(err: SessionStateError) -> Self {
        match err {
            SessionStateError::AlreadyAttached => SessionManagerError::AlreadyAttached,
            SessionStateError::NotAttached => SessionManagerError::NotAttached,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AttachmentMetadata {
    pub client_fingerprint: Option<String>,
}

impl AttachmentMetadata {
    pub fn new(client_fingerprint: Option<String>) -> Self {
        Self { client_fingerprint }
    }
}

#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub id: SessionId,
    pub capability_id: String,
    pub metadata: CapabilityMetadata,
    pub session_mode: SessionMode,
    pub state: SessionStateSnapshot,
}

pub struct SessionHandle {
    pub id: SessionId,
    pub capability_id: String,
    pub metadata: CapabilityMetadata,
    pub session_mode: SessionMode,
    pub client_sender: mpsc::Sender<SessionClientEvent>,
    pub event_receiver: broadcast::Receiver<SessionBroadcastEvent>,
    pub resume_token: String,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SessionManagerInner {
                sessions: RwLock::new(HashMap::new()),
            }),
        }
    }

    pub async fn open_session(
        &self,
        capability: Arc<dyn Capability>,
        parameters: std::collections::HashMap<String, String>,
        client_fingerprint: Option<String>,
    ) -> Result<SessionHandle> {
        let metadata = capability.metadata().clone();
        let session_mode = metadata.session_mode;
        let capability_id = metadata.id.clone();

        let (client_sender, client_receiver) = mpsc::channel::<SessionClientEvent>(64);
        let (server_sender, server_receiver) = mpsc::channel::<SessionServerEvent>(128);
        let (event_tx, event_rx) =
            broadcast::channel::<SessionBroadcastEvent>(DEFAULT_EVENT_CHANNEL_CAPACITY);

        let endpoints = SessionEndpoints {
            inbound: client_receiver,
            outbound: server_sender.clone(),
        };

        let ctx = CapabilityOpenContext {
            metadata: metadata.clone(),
            parameters,
            client_fingerprint,
        };

        capability.open_session(ctx, endpoints).await?;

        let session_id = SessionId::new();
        let state = SessionState::new(DEFAULT_OUTPUT_BUFFER_CAPACITY);
        let resume_token = state.resume_token().to_string();

        let entry = Arc::new(SessionEntry {
            capability_id: capability_id.clone(),
            metadata: metadata.clone(),
            session_mode,
            client_sender: client_sender.clone(),
            _capability: Arc::clone(&capability),
            state: Mutex::new(state),
            event_tx: event_tx.clone(),
        });

        {
            let mut guard = self.inner.sessions.write().unwrap();
            guard.insert(session_id.as_uuid(), Arc::clone(&entry));
        }

        let session_manager = self.clone();
        tokio::spawn(run_session_event_loop(
            session_manager,
            session_id.clone(),
            server_receiver,
            event_tx,
        ));

        Ok(SessionHandle {
            id: session_id,
            capability_id,
            metadata,
            session_mode,
            client_sender,
            event_receiver: event_rx,
            resume_token,
        })
    }

    pub fn get_sender(&self, id: &SessionId) -> Option<mpsc::Sender<SessionClientEvent>> {
        self.inner
            .sessions
            .read()
            .unwrap()
            .get(&id.as_uuid())
            .map(|entry| entry.client_sender.clone())
    }

    pub fn subscribe_events(
        &self,
        id: &SessionId,
    ) -> Result<broadcast::Receiver<SessionBroadcastEvent>, SessionManagerError> {
        let entry = self.get_entry(id)?;
        Ok(entry.event_tx.subscribe())
    }

    pub fn remove(&self, id: &SessionId) {
        self.inner.sessions.write().unwrap().remove(&id.as_uuid());
    }

    pub fn attach(
        &self,
        session_id: &SessionId,
        metadata: AttachmentMetadata,
    ) -> Result<AttachmentState, SessionManagerError> {
        self.with_session_state(session_id, |_entry, state| {
            state
                .try_attach(metadata.client_fingerprint.clone())
                .map_err(Into::into)
        })
    }

    pub fn detach(&self, session_id: &SessionId) -> Result<AttachmentState, SessionManagerError> {
        self.with_session_state(session_id, |_entry, state| {
            state.try_detach().map_err(Into::into)
        })
    }

    pub fn attach_with_token(
        &self,
        session_id: &SessionId,
        metadata: AttachmentMetadata,
        resume_token: &str,
    ) -> Result<(AttachmentState, String), SessionManagerError> {
        let client_fingerprint = metadata.client_fingerprint.clone();
        self.with_session_state(session_id, |_entry, state| {
            if !state.validate_resume_token(resume_token) {
                return Err(SessionManagerError::InvalidResumeToken);
            }
            let attachment = state
                .try_attach(client_fingerprint.clone())
                .map_err(SessionManagerError::from)?;
            let next_token = state.rotate_resume_token();
            Ok((attachment, next_token))
        })
    }

    pub fn touch_heartbeat(&self, session_id: &SessionId) -> Result<(), SessionManagerError> {
        self.with_session_state(session_id, |_entry, state| {
            state.touch_heartbeat().map_err(Into::into)
        })
    }

    pub fn record_server_event(
        &self,
        session_id: &SessionId,
        event: &SessionServerEvent,
    ) -> Result<Option<BufferedOutput>, SessionManagerError> {
        self.with_session_state(session_id, |_entry, state| {
            let frame = match event {
                SessionServerEvent::Output {
                    data,
                    stderr,
                    binary,
                    timestamp_ms,
                } => {
                    let stream = OutputStream::from_stderr_flag(*stderr);
                    Some(state.record_output(stream, data.clone(), *binary, *timestamp_ms))
                }
                _ => None,
            };
            Ok(frame)
        })
    }

    pub fn buffered_output_since(
        &self,
        session_id: &SessionId,
        last_stdout_sequence: Option<u64>,
        last_stderr_sequence: Option<u64>,
    ) -> Result<Vec<BufferedOutput>, SessionManagerError> {
        self.with_session_state(session_id, |_entry, state| {
            Ok(state.outputs_since(last_stdout_sequence, last_stderr_sequence))
        })
    }

    pub fn resume_token(&self, session_id: &SessionId) -> Result<String, SessionManagerError> {
        self.with_session_state(session_id, |_entry, state| {
            Ok(state.resume_token().to_string())
        })
    }

    pub fn rotate_resume_token(
        &self,
        session_id: &SessionId,
    ) -> Result<String, SessionManagerError> {
        self.with_session_state(session_id, |_entry, state| Ok(state.rotate_resume_token()))
    }

    pub fn snapshot(&self, session_id: &SessionId) -> Result<SessionSnapshot, SessionManagerError> {
        let entry = self.get_entry(session_id)?;
        let state = entry.state.lock().unwrap();
        Ok(SessionSnapshot {
            id: session_id.clone(),
            capability_id: entry.capability_id.clone(),
            metadata: entry.metadata.clone(),
            session_mode: entry.session_mode,
            state: state.snapshot(),
        })
    }

    fn get_entry(&self, session_id: &SessionId) -> Result<Arc<SessionEntry>, SessionManagerError> {
        self.inner
            .sessions
            .read()
            .unwrap()
            .get(&session_id.as_uuid())
            .cloned()
            .ok_or(SessionManagerError::NotFound)
    }

    fn with_session_state<F, T>(
        &self,
        session_id: &SessionId,
        op: F,
    ) -> Result<T, SessionManagerError>
    where
        F: FnOnce(&Arc<SessionEntry>, &mut SessionState) -> Result<T, SessionManagerError>,
    {
        let entry = self.get_entry(session_id)?;
        let mut state = entry.state.lock().unwrap();
        op(&entry, &mut state)
    }
}

async fn run_session_event_loop(
    session_manager: SessionManager,
    session_id: SessionId,
    mut receiver: mpsc::Receiver<SessionServerEvent>,
    event_tx: broadcast::Sender<SessionBroadcastEvent>,
) {
    let session_id_str = session_id.to_string();

    while let Some(event) = receiver.recv().await {
        let buffered_output = match session_manager.record_server_event(&session_id, &event) {
            Ok(frame) => frame,
            Err(err) => {
                warn!(
                    session_id = %session_id_str,
                    error = %err,
                    "Failed to record server event"
                );
                None
            }
        };

        let resume_token = if matches!(event, SessionServerEvent::Ready { .. }) {
            match session_manager.resume_token(&session_id) {
                Ok(token) => Some(token),
                Err(err) => {
                    warn!(
                        session_id = %session_id_str,
                        error = %err,
                        "Failed to retrieve resume token for ready event"
                    );
                    None
                }
            }
        } else {
            None
        };

        let broadcast_event = SessionBroadcastEvent {
            event: event.clone(),
            buffered_output,
            resume_token,
        };

        if let Err(err) = event_tx.send(broadcast_event) {
            debug!(
                session_id = %session_id_str,
                error = %err,
                "No active listeners for session event"
            );
        }

        if matches!(event, SessionServerEvent::Closed { .. }) {
            break;
        }
    }

    session_manager.remove(&session_id);
}
