use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::fs;
use std::path::PathBuf;

/// Get the base configuration directory for HandControl
pub fn config_dir() -> Result<PathBuf> {
    let proj_dirs = ProjectDirs::from("", "", "handcontrol")
        .context("Failed to determine project directories")?;

    let config_dir = proj_dirs.config_dir().to_path_buf();

    // Create directory if it doesn't exist
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir).with_context(|| {
            format!(
                "Failed to create config directory: {}",
                config_dir.display()
            )
        })?;
    }

    Ok(config_dir)
}

/// Get the data directory for HandControl (certificates, authorized clients)
pub fn data_dir() -> Result<PathBuf> {
    let proj_dirs = ProjectDirs::from("", "", "handcontrol")
        .context("Failed to determine project directories")?;

    let data_dir = proj_dirs.data_dir().to_path_buf();

    // Create directory if it doesn't exist
    if !data_dir.exists() {
        fs::create_dir_all(&data_dir)
            .with_context(|| format!("Failed to create data directory: {}", data_dir.display()))?;
    }

    Ok(data_dir)
}

/// Get the path to the server certificate
pub fn server_cert_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("server.crt"))
}

/// Get the path to the server private key
pub fn server_key_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("server.key"))
}

/// Get the directory for authorized client certificates
pub fn authorized_clients_dir() -> Result<PathBuf> {
    let dir = config_dir()?.join("authorized_clients");

    // Create directory if it doesn't exist
    if !dir.exists() {
        fs::create_dir_all(&dir).with_context(|| {
            format!(
                "Failed to create authorized_clients directory: {}",
                dir.display()
            )
        })?;
    }

    Ok(dir)
}

/// Get the path to the authorized clients metadata file
pub fn authorized_clients_metadata_path() -> Result<PathBuf> {
    Ok(authorized_clients_dir()?.join("metadata.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paths_not_empty() {
        let config = config_dir().unwrap();
        assert!(!config.as_os_str().is_empty());

        let data = data_dir().unwrap();
        assert!(!data.as_os_str().is_empty());

        let cert = server_cert_path().unwrap();
        assert!(cert.ends_with("server.crt"));

        let key = server_key_path().unwrap();
        assert!(key.ends_with("server.key"));

        let clients = authorized_clients_dir().unwrap();
        assert!(clients.ends_with("authorized_clients"));
    }
}
