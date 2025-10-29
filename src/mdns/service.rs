use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

const SERVICE_TYPE: &str = "_handcontrol._tcp.local.";
const TXT_RECORD_VERSION: &str = "1.0";

/// mDNS service manager for HandControl server
///
/// Advertises the HandControl server on the local network using mDNS/DNS-SD.
/// Clients can discover the server using the _handcontrol._tcp.local. service type.
pub struct MdnsService {
    daemon: Arc<Mutex<Option<ServiceDaemon>>>,
    instance_name: String,
    port: u16,
    server_id: Uuid,
    cert_fingerprint: String,
}

impl MdnsService {
    /// Create a new mDNS service manager
    ///
    /// # Arguments
    /// * `instance_name` - Human-readable instance name (e.g., hostname or user-configured name)
    /// * `port` - TCP port the gRPC server is listening on
    /// * `server_id` - UUID identifying this server instance
    /// * `cert_fingerprint` - SHA256 fingerprint of the server certificate (format: "SHA256:hex")
    pub fn new(
        instance_name: String,
        port: u16,
        server_id: Uuid,
        cert_fingerprint: String,
    ) -> Self {
        Self {
            daemon: Arc::new(Mutex::new(None)),
            instance_name,
            port,
            server_id,
            cert_fingerprint,
        }
    }

    /// Start the mDNS service and register the HandControl service
    ///
    /// This will advertise the server on all network interfaces.
    /// TXT records include:
    /// - version: Protocol version (1.0)
    /// - server_id: UUID of this server
    /// - cert_fingerprint: SHA256 fingerprint of server certificate
    pub fn start(&self) -> Result<()> {
        info!("Starting mDNS service...");

        // Create the mDNS daemon
        let daemon = ServiceDaemon::new()
            .context("Failed to create mDNS service daemon")?;

        // Prepare TXT records
        let mut properties = HashMap::new();
        properties.insert("version".to_string(), TXT_RECORD_VERSION.to_string());
        properties.insert("server_id".to_string(), self.server_id.to_string());
        properties.insert("cert_fingerprint".to_string(), self.cert_fingerprint.clone());

        debug!(
            "mDNS TXT records: version={}, server_id={}, cert_fingerprint={}",
            TXT_RECORD_VERSION, self.server_id, self.cert_fingerprint
        );

        // Create service info
        // Note: mdns-sd will automatically determine the host's IP addresses
        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &self.instance_name,
            &format!("{}.local.", self.instance_name), // hostname
            (), // Use default IP (all interfaces)
            self.port,
            Some(properties),
        )
        .context("Failed to create mDNS service info")?;

        // Register the service
        daemon
            .register(service_info)
            .context("Failed to register mDNS service")?;

        info!(
            "mDNS service registered: {} at port {} ({})",
            self.instance_name, self.port, SERVICE_TYPE
        );

        // Store daemon
        *self.daemon.lock().unwrap() = Some(daemon);

        Ok(())
    }

    /// Stop the mDNS service and unregister
    pub fn stop(&self) -> Result<()> {
        info!("Stopping mDNS service...");

        let mut daemon_guard = self.daemon.lock().unwrap();

        if let Some(daemon) = daemon_guard.take() {
            // Shutdown the daemon (this automatically unregisters all services)
            daemon.shutdown()
                .context("Failed to shutdown mDNS daemon")?;

            info!("mDNS service stopped");
        } else {
            warn!("mDNS service was not running");
        }

        Ok(())
    }

    /// Check if the mDNS service is currently running
    pub fn is_running(&self) -> bool {
        self.daemon.lock().unwrap().is_some()
    }

    /// Update the service registration (for config reload)
    ///
    /// This stops the current service and starts a new one with updated parameters.
    pub fn update(
        &mut self,
        instance_name: String,
        port: u16,
        server_id: Uuid,
        cert_fingerprint: String,
    ) -> Result<()> {
        info!("Updating mDNS service registration...");

        // Stop existing service
        if self.is_running() {
            self.stop().context("Failed to stop existing mDNS service")?;
        }

        // Update parameters
        self.instance_name = instance_name;
        self.port = port;
        self.server_id = server_id;
        self.cert_fingerprint = cert_fingerprint;

        // Restart with new parameters
        self.start().context("Failed to restart mDNS service with new parameters")?;

        Ok(())
    }
}

impl Drop for MdnsService {
    /// Ensure the service is properly unregistered when dropped
    fn drop(&mut self) {
        if self.is_running() {
            if let Err(e) = self.stop() {
                error!("Failed to stop mDNS service in Drop: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mdns_service_creation() {
        let service = MdnsService::new(
            "test-server".to_string(),
            50051,
            Uuid::new_v4(),
            "SHA256:abcd1234".to_string(),
        );

        assert_eq!(service.instance_name, "test-server");
        assert_eq!(service.port, 50051);
        assert!(!service.is_running());
    }

    #[test]
    fn test_mdns_service_lifecycle() {
        let service = MdnsService::new(
            "test-lifecycle".to_string(),
            50052,
            Uuid::new_v4(),
            "SHA256:test".to_string(),
        );

        // Start service
        let result = service.start();
        // Note: This might fail in CI environments without proper mDNS support
        // So we just verify it doesn't panic
        if result.is_ok() {
            assert!(service.is_running());

            // Stop service
            assert!(service.stop().is_ok());
            assert!(!service.is_running());
        }
    }

    #[test]
    fn test_mdns_stop_when_not_running() {
        let service = MdnsService::new(
            "test-not-running".to_string(),
            50053,
            Uuid::new_v4(),
            "SHA256:test".to_string(),
        );

        // Should not error when stopping a non-running service
        assert!(service.stop().is_ok());
    }

    #[test]
    fn test_txt_record_format() {
        // Verify TXT record constants are correct
        assert_eq!(SERVICE_TYPE, "_handcontrol._tcp.local.");
        assert_eq!(TXT_RECORD_VERSION, "1.0");
    }

    #[test]
    fn test_service_type_constant() {
        // Service type must match PRD specification
        assert_eq!(SERVICE_TYPE, "_handcontrol._tcp.local.");
    }
}
