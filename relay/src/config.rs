use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

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
    #[serde(default)]
    pub quic_port: Option<u16>,
    #[serde(default)]
    pub allow_all_audiences: bool,
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
    PathBuf::from("relay.toml")
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
