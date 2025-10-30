use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use tracing::info;
use uuid::Uuid;

use crate::security::certificates::ClientCertificate;

/// Record of an IP connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpConnection {
    pub ip_address: String,
    #[serde(with = "time::serde::rfc3339")]
    pub connected_at: OffsetDateTime,
}

/// Metadata for an authorized client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMetadata {
    pub id: String,
    pub name: String,
    pub cert_fingerprint: String,
    #[serde(with = "time::serde::rfc3339")]
    pub enrolled_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub last_seen: OffsetDateTime,
    #[serde(default)]
    pub ip_history: Vec<IpConnection>,
}

impl ClientMetadata {
    /// Get the most recent IP address
    pub fn last_ip(&self) -> Option<&str> {
        self.ip_history.last().map(|conn| conn.ip_address.as_str())
    }
}

/// Registry of all authorized clients
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRegistry {
    #[serde(default)]
    pub client: Vec<ClientMetadata>,
}

impl Default for ClientRegistry {
    fn default() -> Self {
        Self { client: Vec::new() }
    }
}

/// Manager for authorized client certificates
#[derive(Debug)]
pub struct ClientStore {
    clients_dir: PathBuf,
    metadata_path: PathBuf,
    registry: ClientRegistry,
}

impl ClientStore {
    /// Create a new client store
    pub fn new(clients_dir: PathBuf) -> Result<Self> {
        // Ensure directory exists
        if !clients_dir.exists() {
            fs::create_dir_all(&clients_dir).with_context(|| {
                format!(
                    "Failed to create clients directory: {}",
                    clients_dir.display()
                )
            })?;
        }

        let metadata_path = clients_dir.join("metadata.toml");

        // Load existing registry or create new
        let registry = if metadata_path.exists() {
            Self::load_registry(&metadata_path)?
        } else {
            ClientRegistry::default()
        };

        Ok(Self {
            clients_dir,
            metadata_path,
            registry,
        })
    }

    /// Load registry from file
    fn load_registry(path: &Path) -> Result<ClientRegistry> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read metadata file: {}", path.display()))?;

        toml::from_str(&content)
            .with_context(|| format!("Failed to parse metadata file: {}", path.display()))
    }

    /// Save registry to file
    fn save_registry(&self) -> Result<()> {
        let content = toml::to_string_pretty(&self.registry)
            .context("Failed to serialize client registry")?;

        fs::write(&self.metadata_path, content).with_context(|| {
            format!(
                "Failed to write metadata file: {}",
                self.metadata_path.display()
            )
        })?;

        Ok(())
    }

    /// Add a new authorized client
    pub fn add_client(
        &mut self,
        cert: &ClientCertificate,
        device_name: String,
        ip: Option<IpAddr>,
    ) -> Result<String> {
        let client_id = Uuid::new_v4().to_string();
        let now = OffsetDateTime::now_utc();

        // Save certificate file
        let cert_path = self.clients_dir.join(format!("{}.crt", client_id));
        fs::write(&cert_path, &cert.cert_der).with_context(|| {
            format!(
                "Failed to write client certificate: {}",
                cert_path.display()
            )
        })?;

        // Create IP history if IP provided
        let ip_history = if let Some(ip) = ip {
            vec![IpConnection {
                ip_address: ip.to_string(),
                connected_at: now,
            }]
        } else {
            Vec::new()
        };

        // Add to registry
        let metadata = ClientMetadata {
            id: client_id.clone(),
            name: device_name,
            cert_fingerprint: cert.fingerprint_hex(),
            enrolled_at: now,
            last_seen: now,
            ip_history,
        };

        self.registry.client.push(metadata);
        self.save_registry()?;

        info!(
            "Added client '{}' with ID: {}",
            self.registry.client.last().unwrap().name,
            client_id
        );

        Ok(client_id)
    }

    /// Remove an authorized client
    pub fn remove_client(&mut self, client_id: &str) -> Result<()> {
        // Remove from registry
        let original_len = self.registry.client.len();
        self.registry.client.retain(|c| c.id != client_id);

        if self.registry.client.len() == original_len {
            anyhow::bail!("Client not found: {}", client_id);
        }

        // Remove certificate file
        let cert_path = self.clients_dir.join(format!("{}.crt", client_id));
        if cert_path.exists() {
            fs::remove_file(&cert_path).with_context(|| {
                format!("Failed to remove certificate: {}", cert_path.display())
            })?;
        }

        self.save_registry()?;

        info!("Removed client: {}", client_id);
        Ok(())
    }

    /// Get client by ID
    pub fn get_client(&self, client_id: &str) -> Option<&ClientMetadata> {
        self.registry.client.iter().find(|c| c.id == client_id)
    }

    /// Get client by certificate fingerprint
    pub fn get_client_by_fingerprint(&self, fingerprint: &str) -> Option<&ClientMetadata> {
        self.registry
            .client
            .iter()
            .find(|c| c.cert_fingerprint == fingerprint)
    }

    /// Update last seen time for a client
    pub fn update_last_seen(&mut self, client_id: &str) -> Result<()> {
        self.update_last_seen_with_ip(client_id, None)
    }

    /// Update last seen time and IP address for a client
    pub fn update_last_seen_with_ip(&mut self, client_id: &str, ip: Option<IpAddr>) -> Result<()> {
        if let Some(client) = self.registry.client.iter_mut().find(|c| c.id == client_id) {
            let now = OffsetDateTime::now_utc();
            client.last_seen = now;

            // Add IP to history if provided
            if let Some(ip) = ip {
                let ip_str = ip.to_string();

                // Only add if it's different from the most recent IP or if history is empty
                let should_add = client
                    .ip_history
                    .last()
                    .map(|last| last.ip_address != ip_str)
                    .unwrap_or(true);

                if should_add {
                    client.ip_history.push(IpConnection {
                        ip_address: ip_str,
                        connected_at: now,
                    });

                    // Keep only the last 100 connections to prevent unbounded growth
                    const MAX_IP_HISTORY: usize = 100;
                    if client.ip_history.len() > MAX_IP_HISTORY {
                        client
                            .ip_history
                            .drain(0..client.ip_history.len() - MAX_IP_HISTORY);
                    }
                }
            }

            self.save_registry()?;
            Ok(())
        } else {
            anyhow::bail!("Client not found: {}", client_id);
        }
    }

    /// List all authorized clients
    pub fn list_clients(&self) -> &[ClientMetadata] {
        &self.registry.client
    }

    /// Load client certificate from file
    pub fn load_client_certificate(&self, client_id: &str) -> Result<ClientCertificate> {
        let cert_path = self.clients_dir.join(format!("{}.crt", client_id));
        let cert_der = fs::read(&cert_path).with_context(|| {
            format!("Failed to read client certificate: {}", cert_path.display())
        })?;

        Ok(ClientCertificate::from_der(cert_der))
    }

    /// Check if a certificate is authorized
    pub fn is_authorized(&self, cert_fingerprint: &str) -> bool {
        self.get_client_by_fingerprint(cert_fingerprint).is_some()
    }

    /// Get all authorized certificate fingerprints
    pub fn authorized_fingerprints(&self) -> HashMap<String, String> {
        self.registry
            .client
            .iter()
            .map(|c| (c.cert_fingerprint.clone(), c.id.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::certificates::ServerCertificate;
    use tempfile::TempDir;

    #[test]
    fn test_client_store_new() {
        let temp_dir = TempDir::new().unwrap();
        let clients_dir = temp_dir.path().join("clients");

        let store = ClientStore::new(clients_dir.clone()).unwrap();
        assert!(clients_dir.exists());
        assert_eq!(store.list_clients().len(), 0);
    }

    #[test]
    fn test_add_and_get_client() {
        let temp_dir = TempDir::new().unwrap();
        let mut store = ClientStore::new(temp_dir.path().join("clients")).unwrap();

        // Create a test certificate
        let cert = ServerCertificate::generate().unwrap();
        let client_cert = ClientCertificate::from_der(cert.cert_der);

        // Add client
        let client_id = store
            .add_client(&client_cert, "Test Device".to_string(), None)
            .unwrap();

        // Get by ID
        let metadata = store.get_client(&client_id).unwrap();
        assert_eq!(metadata.name, "Test Device");
        assert_eq!(metadata.cert_fingerprint, client_cert.fingerprint_hex());

        // Get by fingerprint
        let metadata2 = store
            .get_client_by_fingerprint(&client_cert.fingerprint_hex())
            .unwrap();
        assert_eq!(metadata2.id, client_id);
    }

    #[test]
    fn test_remove_client() {
        let temp_dir = TempDir::new().unwrap();
        let mut store = ClientStore::new(temp_dir.path().join("clients")).unwrap();

        let cert = ServerCertificate::generate().unwrap();
        let client_cert = ClientCertificate::from_der(cert.cert_der);

        let client_id = store
            .add_client(&client_cert, "Test Device".to_string(), None)
            .unwrap();
        assert_eq!(store.list_clients().len(), 1);

        store.remove_client(&client_id).unwrap();
        assert_eq!(store.list_clients().len(), 0);
        assert!(store.get_client(&client_id).is_none());
    }

    #[test]
    fn test_update_last_seen() {
        let temp_dir = TempDir::new().unwrap();
        let mut store = ClientStore::new(temp_dir.path().join("clients")).unwrap();

        let cert = ServerCertificate::generate().unwrap();
        let client_cert = ClientCertificate::from_der(cert.cert_der);

        let client_id = store
            .add_client(&client_cert, "Test Device".to_string(), None)
            .unwrap();
        let original_last_seen = store.get_client(&client_id).unwrap().last_seen;

        // Wait a tiny bit to ensure time difference
        std::thread::sleep(std::time::Duration::from_millis(10));

        store.update_last_seen(&client_id).unwrap();
        let new_last_seen = store.get_client(&client_id).unwrap().last_seen;

        assert!(new_last_seen > original_last_seen);
    }

    #[test]
    fn test_is_authorized() {
        let temp_dir = TempDir::new().unwrap();
        let mut store = ClientStore::new(temp_dir.path().join("clients")).unwrap();

        let cert = ServerCertificate::generate().unwrap();
        let client_cert = ClientCertificate::from_der(cert.cert_der);

        assert!(!store.is_authorized(&client_cert.fingerprint_hex()));

        store
            .add_client(&client_cert, "Test Device".to_string(), None)
            .unwrap();

        assert!(store.is_authorized(&client_cert.fingerprint_hex()));
    }

    #[test]
    fn test_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let clients_dir = temp_dir.path().join("clients");

        let cert = ServerCertificate::generate().unwrap();
        let client_cert = ClientCertificate::from_der(cert.cert_der.clone());

        // Add client in first store
        let client_id = {
            let mut store = ClientStore::new(clients_dir.clone()).unwrap();
            store
                .add_client(&client_cert, "Test Device".to_string(), None)
                .unwrap()
        };

        // Load from disk in second store
        let store2 = ClientStore::new(clients_dir).unwrap();
        assert_eq!(store2.list_clients().len(), 1);
        assert!(store2.get_client(&client_id).is_some());
        assert!(store2.is_authorized(&client_cert.fingerprint_hex()));
    }
}
