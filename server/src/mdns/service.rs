use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

const SERVICE_TYPE: &str = "_handcontrol._tcp.local.";
#[cfg(any(target_os = "macos", target_os = "linux"))]
const SERVICE_TYPE_NO_DOMAIN: &str = "_handcontrol._tcp";
const TXT_RECORD_VERSION: &str = "1.0";
const DEFAULT_HOST_LABEL: &str = "handcontrol";

/// mDNS service manager for HandControl server
///
/// Advertises the HandControl server on the local network using mDNS/DNS-SD.
/// The concrete backend is chosen at runtime:
///   * macOS -> delegates to the system Bonjour daemon via `dns-sd`
///   * Linux -> prefers Avahi (`avahi-publish-service`) when available
///   * all other platforms -> embedded `mdns-sd` crate
pub struct MdnsService {
    handle: Arc<Mutex<Option<platform::Handle>>>,
    instance_name: String,
    port: u16,
    server_id: Uuid,
    cert_fingerprint: String,
}

impl MdnsService {
    /// Create a new mDNS service manager
    pub fn new(
        instance_name: String,
        port: u16,
        server_id: Uuid,
        cert_fingerprint: String,
    ) -> Self {
        Self {
            handle: Arc::new(Mutex::new(None)),
            instance_name,
            port,
            server_id,
            cert_fingerprint,
        }
    }

    /// Start the mDNS service and register the HandControl service
    pub fn start(&self) -> Result<()> {
        info!("Starting mDNS service...");

        if self.is_running() {
            warn!("mDNS service already running, restarting with updated configuration");
            self.stop()
                .context("Failed to stop existing mDNS service before restart")?;
        }

        let txt_records = self.build_txt_records();
        let (handle, backend_name) = platform::start(&self.instance_name, self.port, &txt_records)
            .context("Failed to activate mDNS backend")?;

        debug!("mDNS TXT records: {:?}", txt_records);
        info!(
            "mDNS service registered: {} at port {} ({}) via {}",
            self.instance_name, self.port, SERVICE_TYPE, backend_name
        );

        *self.handle.lock().unwrap() = Some(handle);

        Ok(())
    }

    /// Stop the mDNS service and unregister
    pub fn stop(&self) -> Result<()> {
        info!("Stopping mDNS service...");

        let mut guard = self.handle.lock().unwrap();

        if let Some(handle) = guard.take() {
            handle.stop()?;
            info!("mDNS service stopped");
        } else {
            warn!("mDNS service was not running");
        }

        Ok(())
    }

    /// Check if the mDNS service is currently running
    pub fn is_running(&self) -> bool {
        self.handle.lock().unwrap().is_some()
    }

    /// Update the service registration (for config reload)
    pub fn update(
        &mut self,
        instance_name: String,
        port: u16,
        server_id: Uuid,
        cert_fingerprint: String,
    ) -> Result<()> {
        info!("Updating mDNS service registration...");

        self.stop()
            .context("Failed to stop existing mDNS service")?;

        self.instance_name = instance_name;
        self.port = port;
        self.server_id = server_id;
        self.cert_fingerprint = cert_fingerprint;

        self.start()
            .context("Failed to restart mDNS service with new parameters")?;

        Ok(())
    }

    fn build_txt_records(&self) -> Vec<(String, String)> {
        vec![
            ("version".to_string(), TXT_RECORD_VERSION.to_string()),
            ("server_id".to_string(), self.server_id.to_string()),
            (
                "cert_fingerprint".to_string(),
                self.cert_fingerprint.clone(),
            ),
        ]
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

fn derive_host_name() -> String {
    let label = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .map(|raw| sanitize_host_label(&raw))
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| DEFAULT_HOST_LABEL.to_string());

    format!("{}.local.", label)
}

fn sanitize_host_label(input: &str) -> String {
    let mut label: String = input
        .chars()
        .map(|c| match c {
            c if c.is_ascii_alphanumeric() => c.to_ascii_lowercase(),
            '-' => '-',
            '_' => '-',
            c if c.is_whitespace() => '-',
            _ => '\0',
        })
        .filter(|c| *c != '\0')
        .collect();

    while label.starts_with('-') {
        label.remove(0);
    }
    while label.ends_with('-') {
        label.pop();
    }

    if label.len() > 63 {
        label.truncate(63);
    }

    label
}

mod platform {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use super::SERVICE_TYPE_NO_DOMAIN;
    use super::{Result, SERVICE_TYPE, derive_host_name};
    use anyhow::Context;
    use mdns_sd::{ServiceDaemon, ServiceInfo};
    use std::collections::HashMap;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::io::{self, Read};
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::process::{Child, Command, Stdio};
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::thread;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::time::Duration;
    use tracing::{info, warn};

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use anyhow::bail;

    pub enum Handle {
        Internal(ServiceDaemon),
        #[cfg(target_os = "macos")]
        Bonjour {
            child: Child,
        },
        #[cfg(target_os = "linux")]
        Avahi {
            child: Child,
        },
    }

    impl Handle {
        pub fn backend_name(&self) -> &'static str {
            match self {
                Handle::Internal(..) => "embedded-mdns-sd",
                #[cfg(target_os = "macos")]
                Handle::Bonjour { .. } => "dns-sd",
                #[cfg(target_os = "linux")]
                Handle::Avahi { .. } => "avahi",
            }
        }

        pub fn stop(self) -> Result<()> {
            match self {
                Handle::Internal(daemon) => {
                    let receiver = daemon
                        .shutdown()
                        .context("Failed to shutdown mDNS daemon")?;
                    drop(receiver);
                    Ok(())
                }
                #[cfg(target_os = "macos")]
                Handle::Bonjour { mut child } => stop_child(&mut child, "dns-sd"),
                #[cfg(target_os = "linux")]
                Handle::Avahi { mut child } => stop_child(&mut child, "avahi-publish-service"),
            }
        }
    }

    pub fn start(
        instance_name: &str,
        port: u16,
        txt_records: &[(String, String)],
    ) -> Result<(Handle, &'static str)> {
        #[cfg(target_os = "macos")]
        {
            let handle = start_macos(instance_name, port, txt_records)?;
            let backend_name = handle.backend_name();
            return Ok((handle, backend_name));
        }

        #[cfg(target_os = "linux")]
        {
            match start_avahi(instance_name, port, txt_records) {
                Ok(handle) => {
                    let backend_name = handle.backend_name();
                    return Ok((handle, backend_name));
                }
                Err(err) => {
                    warn!(
                        "Failed to publish mDNS via Avahi (falling back to embedded mdns-sd): {}",
                        err
                    );
                }
            }
        }

        let handle = start_internal(instance_name, port, txt_records)?;
        let backend_name = handle.backend_name();
        Ok((handle, backend_name))
    }

    #[cfg(target_os = "macos")]
    fn start_macos(
        instance_name: &str,
        port: u16,
        txt_records: &[(String, String)],
    ) -> Result<Handle> {
        let mut args = vec![
            "-R".to_string(),
            instance_name.to_string(),
            SERVICE_TYPE_NO_DOMAIN.to_string(),
            "local".to_string(),
            port.to_string(),
        ];

        for (key, value) in txt_records {
            args.push(format!("{}={}", key, value));
        }

        info!(
            "Registering mDNS via system dns-sd: instance={} port={} txt={:?}",
            instance_name, port, txt_records
        );

        let mut child = Command::new("dns-sd")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn dns-sd registration command")?;

        wait_for_child_ready(&mut child, "dns-sd")?;

        Ok(Handle::Bonjour { child })
    }

    #[cfg(target_os = "linux")]
    fn start_avahi(
        instance_name: &str,
        port: u16,
        txt_records: &[(String, String)],
    ) -> Result<Handle> {
        let mut args = vec![
            instance_name.to_string(),
            SERVICE_TYPE_NO_DOMAIN.to_string(),
            port.to_string(),
        ];

        for (key, value) in txt_records {
            args.push(format!("{}={}", key, value));
        }

        info!(
            "Attempting Avahi registration: instance={} port={} txt={:?}",
            instance_name, port, txt_records
        );

        let mut child = Command::new("avahi-publish-service")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn avahi-publish-service command")?;

        match wait_for_child_ready(&mut child, "avahi-publish-service") {
            Ok(()) => Ok(Handle::Avahi { child }),
            Err(err) => Err(err),
        }
    }

    fn start_internal(
        instance_name: &str,
        port: u16,
        txt_records: &[(String, String)],
    ) -> Result<Handle> {
        info!(
            "Registering mDNS via embedded mdns-sd daemon: instance={} port={} txt={:?}",
            instance_name, port, txt_records
        );

        let daemon = ServiceDaemon::new().context("Failed to create mDNS service daemon")?;

        let mut properties = HashMap::new();
        for (key, value) in txt_records {
            properties.insert(key.clone(), value.clone());
        }

        let host_name = derive_host_name();

        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            instance_name,
            &host_name,
            (),
            port,
            Some(properties),
        )
        .context("Failed to create mDNS service info")?
        .enable_addr_auto();

        daemon
            .register(service_info)
            .context("Failed to register mDNS service")?;

        Ok(Handle::Internal(daemon))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn wait_for_child_ready(child: &mut Child, name: &str) -> Result<()> {
        thread::sleep(Duration::from_millis(200));

        if let Some(status) = child
            .try_wait()
            .context(format!("Failed to check {} process status", name))?
        {
            let mut stderr_output = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                let _ = stderr.read_to_string(&mut stderr_output);
            }
            bail!(
                "{} exited early with status {}{}",
                name,
                status,
                if stderr_output.trim().is_empty() {
                    String::new()
                } else {
                    format!(": {}", stderr_output.trim())
                }
            );
        }

        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn stop_child(child: &mut Child, name: &str) -> Result<()> {
        if let Err(error) = child.kill() {
            if error.kind() != io::ErrorKind::InvalidInput {
                return Err(error).context(format!("Failed to terminate {} process", name));
            }
        }

        child
            .wait()
            .context(format!("Failed to wait for {} process shutdown", name))?;

        Ok(())
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

        let result = service.start();
        if result.is_ok() {
            assert!(service.is_running());

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

        assert!(service.stop().is_ok());
    }

    #[test]
    fn test_txt_record_format() {
        assert_eq!(SERVICE_TYPE, "_handcontrol._tcp.local.");
        assert_eq!(TXT_RECORD_VERSION, "1.0");
    }

    #[test]
    fn test_service_type_constant() {
        assert_eq!(SERVICE_TYPE, "_handcontrol._tcp.local.");
    }

    #[test]
    fn test_sanitize_host_label() {
        assert_eq!(sanitize_host_label("My Computer"), "my-computer");
        assert_eq!(sanitize_host_label("HandControl"), "handcontrol");
        assert_eq!(
            sanitize_host_label("  --Invalid Hostname!! "),
            "invalid-hostname"
        );
        let long_input = "a".repeat(80);
        assert_eq!(sanitize_host_label(&long_input), "a".repeat(63));
    }
}
