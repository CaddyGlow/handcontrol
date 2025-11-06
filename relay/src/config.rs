use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const RELAY_CONFIG_DIR_ENV: &str = "HANDCONTROL_RELAY_CONFIG_DIR";
const SHARED_CONFIG_DIR_ENV: &str = "HANDCONTROL_CONFIG_DIR";
const RELAY_CONFIG_FILE: &str = "relay.toml";

#[derive(Debug, Deserialize)]
pub struct RelayConfig {
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_handshake_timeout_seconds")]
    pub handshake_timeout_seconds: u64,
    #[serde(default)]
    pub public_hostname: Option<String>,
    #[serde(default)]
    pub registration_secrets: HashMap<Uuid, String>,
    #[serde(default)]
    pub tls_cert_path: Option<PathBuf>,
    #[serde(default)]
    pub tls_key_path: Option<PathBuf>,
}

pub fn load_config(path: impl AsRef<Path>) -> Result<RelayConfig> {
    let path = path.as_ref();
    let contents =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let config: RelayConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse relay config {}", path.display()))?;
    Ok(config)
}

pub fn default_config_path() -> PathBuf {
    if let Some(dir) = env_config_dir(RELAY_CONFIG_DIR_ENV) {
        return dir.join(RELAY_CONFIG_FILE);
    }

    if let Some(dir) = env_config_dir(SHARED_CONFIG_DIR_ENV) {
        return dir.join(RELAY_CONFIG_FILE);
    }

    PathBuf::from(RELAY_CONFIG_FILE)
}

fn env_config_dir(var: &str) -> Option<PathBuf> {
    let value = std::env::var_os(var)?;
    let dir = PathBuf::from(value);
    if dir.as_os_str().is_empty() {
        panic!("{var} is set but empty; provide a valid directory path");
    }
    fs::create_dir_all(&dir).unwrap_or_else(|err| {
        panic!(
            "Failed to create config directory {} from {var}: {err}",
            dir.display()
        )
    });
    Some(dir)
}

fn default_bind_address() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    443
}

fn default_handshake_timeout_seconds() -> u64 {
    5
}
