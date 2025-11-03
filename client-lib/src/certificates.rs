use crate::config;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const CERT_ROOT_DIR: &str = "client-certs";
const CLIENT_CERT_FILE: &str = "client.crt";
const CLIENT_KEY_FILE: &str = "client.key";
const SERVER_PIN_FILE: &str = "server.crt.pinned";

/// Filesystem locations for a server's certificate material.
#[derive(Debug, Clone)]
pub struct CertificatePaths {
    pub dir: PathBuf,
    pub client_cert: PathBuf,
    pub client_key: PathBuf,
    pub server_fingerprint: PathBuf,
}

impl CertificatePaths {
    /// Compute certificate paths for the given server, creating directories as needed.
    pub fn for_server(id: &Uuid) -> Result<Self> {
        let mut base_dir = config::config_dir()?;
        base_dir.push(CERT_ROOT_DIR);
        fs::create_dir_all(&base_dir).with_context(|| {
            format!(
                "Failed to create certificate root directory {}",
                base_dir.display()
            )
        })?;
        apply_dir_permissions(&base_dir)?;

        let mut server_dir = base_dir;
        server_dir.push(id.to_string());
        fs::create_dir_all(&server_dir)
            .with_context(|| format!("Failed to create certificate directory for server {}", id))?;
        apply_dir_permissions(&server_dir)?;

        let paths = Self {
            client_cert: server_dir.join(CLIENT_CERT_FILE),
            client_key: server_dir.join(CLIENT_KEY_FILE),
            server_fingerprint: server_dir.join(SERVER_PIN_FILE),
            dir: server_dir,
        };

        paths.apply_file_permissions()?;
        Ok(paths)
    }

    /// Whether both client cert and key exist on disk.
    pub fn credentials_exist(&self) -> bool {
        self.client_cert.exists() && self.client_key.exists()
    }

    /// Persist freshly generated client credentials to disk.
    pub fn write_client_credentials(&self, cert_pem: &str, key_pem: &str) -> Result<()> {
        fs::write(&self.client_cert, cert_pem)
            .with_context(|| format!("Failed to write {}", self.client_cert.display()))?;
        apply_file_permissions(&self.client_cert, 0o644)?;

        fs::write(&self.client_key, key_pem)
            .with_context(|| format!("Failed to write {}", self.client_key.display()))?;
        apply_file_permissions(&self.client_key, 0o600)?;
        Ok(())
    }

    /// Load stored client credentials (PEM-encoded certificate and private key).
    pub fn load_client_credentials(&self) -> Result<(Vec<u8>, Vec<u8>)> {
        let cert = fs::read(&self.client_cert)
            .with_context(|| format!("Failed to read {}", self.client_cert.display()))?;
        let key = fs::read(&self.client_key)
            .with_context(|| format!("Failed to read {}", self.client_key.display()))?;
        Ok((cert, key))
    }

    /// Apply recommended permissions to key/cert files if they already exist.
    pub fn apply_file_permissions(&self) -> Result<()> {
        if self.client_key.exists() {
            apply_file_permissions(&self.client_key, 0o600)?;
        }
        if self.client_cert.exists() {
            apply_file_permissions(&self.client_cert, 0o644)?;
        }
        if self.server_fingerprint.exists() {
            apply_file_permissions(&self.server_fingerprint, 0o644)?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn apply_dir_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = fs::Permissions::from_mode(0o700);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn apply_dir_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn apply_file_permissions(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = fs::Permissions::from_mode(mode);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn apply_file_permissions(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}
