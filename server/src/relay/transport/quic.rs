use anyhow::{Result, bail};
use async_trait::async_trait;

use super::{ControlConnectParams, ControlSession, RelayTransport, TunnelConnectParams};

/// Placeholder QUIC transport for the relay client. The server-side QUIC
/// listener will be implemented alongside the relay endpoint.
#[derive(Debug, Default, Clone)]
pub struct QuicTransport;

#[async_trait]
impl RelayTransport for QuicTransport {
    async fn connect_control(&self, params: ControlConnectParams) -> Result<ControlSession> {
        let _ = params.quic_port;
        bail!(
            "Server relay QUIC transport not implemented yet (attempted {})",
            params.register_url
        );
    }

    async fn spawn_tunnel(&self, params: TunnelConnectParams) -> Result<()> {
        let _ = params.quic_port;
        bail!(
            "Server relay QUIC transport cannot open tunnel {} (not implemented yet)",
            params.tunnel_id
        );
    }
}
