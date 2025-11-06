use super::{
    ControlConnectParams, ControlSession, RelayTransport, TunnelAttachParams, TunnelConnection,
};
use anyhow::{bail, Result};
use async_trait::async_trait;

/// Placeholder QUIC transport implementation. The actual handshake logic will be added once the
/// relay exposes a QUIC endpoint.
#[derive(Debug, Default, Clone)]
pub struct QuicTransport;

#[async_trait]
impl RelayTransport for QuicTransport {
    async fn connect_control(&self, _params: ControlConnectParams) -> Result<ControlSession> {
        bail!("QUIC transport not implemented yet");
    }

    async fn attach_tunnel(
        &self,
        _session: ControlSession,
        _params: TunnelAttachParams,
    ) -> Result<TunnelConnection> {
        bail!("QUIC transport not implemented yet");
    }
}
