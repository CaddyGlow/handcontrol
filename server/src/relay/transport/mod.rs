use std::{net::SocketAddr, pin::Pin, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use futures_util::Stream;

pub mod websocket;

/// TLS behavior required by the underlying transport implementation.
#[derive(Clone, Copy, Debug)]
pub struct TlsOptions {
    pub allow_self_signed: bool,
    pub pinned_cert_sha256: Option<[u8; 32]>,
}

/// Parameters needed to establish the relay control channel.
#[derive(Clone, Debug)]
pub struct ControlConnectParams {
    pub register_url: String,
    pub subprotocol: String,
    pub tls: TlsOptions,
}

/// Parameters required to establish and bridge an individual relay tunnel.
#[derive(Clone, Debug)]
pub struct TunnelConnectParams {
    pub tunnel_url: String,
    pub tunnel_id: String,
    pub subprotocol: String,
    pub local_endpoint: SocketAddr,
    pub tls: TlsOptions,
}

/// Transport-agnostic representation of frames exchanged on the control channel.
#[derive(Debug)]
pub enum ControlFrame {
    Text(String),
    Binary(Vec<u8>),
    Close,
    Ping(Vec<u8>),
    Pong(Vec<u8>),
}

/// Trait representing the write-half of the control channel.
#[async_trait]
pub trait ControlSink: Send + Sync {
    async fn send_text(&self, message: &str) -> Result<()>;
    async fn send_binary(&self, payload: &[u8]) -> Result<()>;
    async fn send_ping(&self) -> Result<()>;
}

/// Owning wrapper around a transport-specific control session.
pub struct ControlSession {
    sink: Arc<dyn ControlSink>,
    stream: Pin<Box<dyn Stream<Item = Result<ControlFrame>> + Send>>,
}

impl ControlSession {
    pub fn new(
        sink: Arc<dyn ControlSink>,
        stream: Pin<Box<dyn Stream<Item = Result<ControlFrame>> + Send>>,
    ) -> Self {
        Self { sink, stream }
    }

    pub fn sink(&self) -> Arc<dyn ControlSink> {
        Arc::clone(&self.sink)
    }

    /// Retrieve a single frame from the control stream.
    pub async fn next(&mut self) -> Option<Result<ControlFrame>> {
        use futures_util::StreamExt;

        self.stream.next().await
    }
}

/// Abstraction over the underlying relay transport (WebSocket, QUIC, etc.).
#[async_trait]
pub trait RelayTransport: Send + Sync {
    async fn connect_control(&self, params: ControlConnectParams) -> Result<ControlSession>;
    async fn spawn_tunnel(&self, params: TunnelConnectParams) -> Result<()>;
}
