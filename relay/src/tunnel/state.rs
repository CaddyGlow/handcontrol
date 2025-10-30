use std::time::{Duration, Instant};

/// Tracks the readiness handshake for a relay tunnel.
#[derive(Debug)]
pub struct TunnelState {
    created_at: Instant,
    client_ready_at: Option<Instant>,
    server_ready_at: Option<Instant>,
}

impl TunnelState {
    /// Creates a fresh tunnel in `Pending` state.
    pub fn new(now: Instant) -> Self {
        Self {
            created_at: now,
            client_ready_at: None,
            server_ready_at: None,
        }
    }

    /// Marks the client half of the tunnel as ready.
    ///
    /// Returns `true` if both halves are now ready.
    pub fn mark_client_ready(&mut self, now: Instant) -> bool {
        self.client_ready_at = Some(now);
        self.is_fully_ready()
    }

    /// Marks the server half of the tunnel as ready.
    ///
    /// Returns `true` if both halves are now ready.
    pub fn mark_server_ready(&mut self, now: Instant) -> bool {
        self.server_ready_at = Some(now);
        self.is_fully_ready()
    }

    /// Returns `true` once both client and server have signalled readiness.
    pub fn is_fully_ready(&self) -> bool {
        self.client_ready_at.is_some() && self.server_ready_at.is_some()
    }

    /// Returns `true` when the handshake has not completed within the provided timeout.
    pub fn is_expired(&self, now: Instant, timeout: Duration) -> bool {
        now.duration_since(self.created_at) > timeout && !self.is_fully_ready()
    }

    /// Returns the instant when the client first signalled readiness.
    pub fn client_ready_at(&self) -> Option<Instant> {
        self.client_ready_at
    }

    /// Returns the instant when the server first signalled readiness.
    pub fn server_ready_at(&self) -> Option<Instant> {
        self.server_ready_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_requires_both_parties() {
        let base = Instant::now();
        let mut state = TunnelState::new(base);

        assert!(!state.is_fully_ready());

        let ready = state.mark_client_ready(base + Duration::from_millis(10));
        assert!(!ready);
        assert!(!state.is_fully_ready());
        assert!(state.client_ready_at().is_some());
        assert!(state.server_ready_at().is_none());

        let ready = state.mark_server_ready(base + Duration::from_millis(15));
        assert!(ready);
        assert!(state.is_fully_ready());
        assert!(state.server_ready_at().is_some());
    }

    #[test]
    fn tunnel_expires_if_second_party_never_arrives() {
        let base = Instant::now();
        let mut state = TunnelState::new(base);

        state.mark_client_ready(base + Duration::from_millis(5));
        let expired = state.is_expired(base + Duration::from_secs(6), Duration::from_secs(5));
        assert!(expired, "tunnel should expire if server never connects");
    }

    #[test]
    fn tunnel_does_not_expire_after_activation() {
        let base = Instant::now();
        let mut state = TunnelState::new(base);

        state.mark_client_ready(base + Duration::from_millis(5));
        state.mark_server_ready(base + Duration::from_millis(10));

        let expired = state.is_expired(base + Duration::from_secs(60), Duration::from_secs(5));
        assert!(
            !expired,
            "active tunnels should not be considered expired even after timeout window"
        );
    }
}
