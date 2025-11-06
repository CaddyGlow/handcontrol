use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use futures_util::Stream;
use tokio::io::DuplexStream;

pub mod quic;
pub mod websocket;

/// TLS configuration shared across relay transports.
#[derive(Clone, Copy, Debug)]
pub struct TlsOptions {
    pub allow_self_signed: bool,
    pub pinned_cert_sha256: Option<[u8; 32]>,
}

/// Parameters used to establish the initial relay control connection.
#[derive(Clone, Debug)]
pub struct ControlConnectParams {
    pub url: String,
    pub host: String,
    pub port: u16,
    pub subprotocol: Option<String>,
    pub tls: TlsOptions,
    pub fallback_on_subprotocol_error: bool,
}

/// Parameters provided when the relay promotes the control channel into a tunnel.
#[derive(Clone, Debug)]
pub struct TunnelAttachParams {
    pub tunnel_id: String,
    pub role: &'static str,
}

/// Logical frames observed on the control channel.
#[derive(Debug)]
pub enum ControlFrame {
    Text(String),
    Binary(Vec<u8>),
    Close,
    Ping(Vec<u8>),
    Pong(Vec<u8>),
}

/// Trait representing the write half of the control channel.
#[async_trait]
pub trait ControlSink: Send + Sync {
    async fn send_text(&self, message: &str) -> Result<()>;
    async fn send_binary(&self, payload: &[u8]) -> Result<()>;
    async fn send_pong(&self, payload: &[u8]) -> Result<()>;
    async fn send_close(&self) -> Result<()>;
}

/// Transport-agnostic control session wrapper.
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

    pub async fn next(&mut self) -> Option<Result<ControlFrame>> {
        use futures_util::StreamExt;
        self.stream.next().await
    }

    pub fn into_parts(
        self,
    ) -> (
        Arc<dyn ControlSink>,
        Pin<Box<dyn Stream<Item = Result<ControlFrame>> + Send>>,
    ) {
        (self.sink, self.stream)
    }
}

/// Result of attaching a tunnel to the control channel.
pub struct TunnelConnection {
    pub stream: DuplexStream,
}

/// Abstraction over relay transports on the client.
#[async_trait]
pub trait RelayTransport: Send + Sync {
    async fn connect_control(&self, params: ControlConnectParams) -> Result<ControlSession>;
    async fn attach_tunnel(
        &self,
        session: ControlSession,
        params: TunnelAttachParams,
    ) -> Result<TunnelConnection>;
}
