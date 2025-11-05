use super::{SessionClientEvent, SessionEndpoints, SessionId, SessionServerEvent};
use crate::capabilities::{Capability, CapabilityMetadata, CapabilityOpenContext, SessionMode};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct SessionManager {
    inner: Arc<SessionManagerInner>,
}

struct SessionManagerInner {
    sessions: RwLock<HashMap<uuid::Uuid, SessionEntry>>,
}

struct SessionEntry {
    _capability_id: String,
    _metadata: CapabilityMetadata,
    _session_mode: SessionMode,
    client_sender: mpsc::Sender<SessionClientEvent>,
    _capability: Arc<dyn Capability>,
}

pub struct SessionHandle {
    pub id: SessionId,
    pub capability_id: String,
    pub metadata: CapabilityMetadata,
    pub session_mode: SessionMode,
    pub client_sender: mpsc::Sender<SessionClientEvent>,
    pub server_receiver: mpsc::Receiver<SessionServerEvent>,
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

        {
            let mut guard = self.inner.sessions.write().unwrap();
            guard.insert(
                session_id.as_uuid(),
                SessionEntry {
                    _capability_id: capability_id.clone(),
                    _metadata: metadata.clone(),
                    _session_mode: session_mode,
                    client_sender: client_sender.clone(),
                    _capability: Arc::clone(&capability),
                },
            );
        }

        Ok(SessionHandle {
            id: session_id,
            capability_id,
            metadata,
            session_mode,
            client_sender,
            server_receiver,
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

    pub fn remove(&self, id: &SessionId) {
        self.inner.sessions.write().unwrap().remove(&id.as_uuid());
    }
}
