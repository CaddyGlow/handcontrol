use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub server: ServerConfig,
    pub security: SecurityConfig,
    #[serde(default)]
    pub command: Vec<CommandConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    #[serde(default = "default_mdns_service_name")]
    pub mdns_service_name: String,
    pub mdns_instance_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecurityConfig {
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    pub authorized_clients_dir: Option<String>,
    #[serde(default = "default_enrollment_token_ttl")]
    pub enrollment_token_ttl: u64,
    #[serde(default)]
    pub enrollment: EnrollmentConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct EnrollmentConfig {
    #[serde(default = "default_true")]
    pub qr_code_enabled: bool,
    #[serde(default = "default_true")]
    pub approval_enabled: bool,
    #[serde(default = "default_approval_timeout")]
    pub approval_timeout_seconds: u64,
    #[serde(default = "default_true")]
    pub approval_notification: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommandConfig {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub shell: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_command_timeout")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub parameters: Vec<ParameterConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParameterConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
    pub description: Option<String>,
    pub min: Option<i32>,
    pub max: Option<i32>,
    pub default: Option<String>,
    #[serde(default)]
    pub options: Vec<String>,
    pub validation: Option<String>,
    pub label_on: Option<String>,
    pub label_off: Option<String>,
    pub step: Option<i32>,
}

// Default value functions
fn default_port() -> u16 {
    50051
}

fn default_bind_address() -> String {
    "0.0.0.0".to_string()
}

fn default_mdns_service_name() -> String {
    "handcontrol".to_string()
}

fn default_enrollment_token_ttl() -> u64 {
    300 // 5 minutes
}

fn default_true() -> bool {
    true
}

fn default_approval_timeout() -> u64 {
    60 // 60 seconds
}

fn default_command_timeout() -> u64 {
    30 // 30 seconds
}

/// Load configuration from a TOML file
pub fn load_config<P: AsRef<Path>>(path: P) -> Result<Config> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let mut config: Config = toml::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    // Expand ~ in paths
    expand_paths(&mut config)?;

    Ok(config)
}

/// Load configuration from a TOML string (for testing)
pub fn load_config_from_str(content: &str) -> Result<Config> {
    let mut config: Config = toml::from_str(content)
        .context("Failed to parse config from string")?;

    expand_paths(&mut config)?;

    Ok(config)
}

/// Expand ~ in paths to home directory
fn expand_paths(config: &mut Config) -> Result<()> {
    let home = dirs::home_dir().context("Failed to determine home directory")?;

    if let Some(ref mut path) = config.security.cert_path {
        *path = expand_tilde(path, &home);
    }

    if let Some(ref mut path) = config.security.key_path {
        *path = expand_tilde(path, &home);
    }

    if let Some(ref mut path) = config.security.authorized_clients_dir {
        *path = expand_tilde(path, &home);
    }

    Ok(())
}

fn expand_tilde(path: &str, home: &Path) -> String {
    if path.starts_with("~/") {
        home.join(&path[2..]).to_string_lossy().to_string()
    } else {
        path.to_string()
    }
}

/// Get the default config file path for the platform
pub fn default_config_path() -> Result<PathBuf> {
    let config_dir = crate::storage::paths::config_dir()?;
    Ok(config_dir.join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let toml = r#"
            [server]
            port = 50051

            [security]
        "#;

        let config = load_config_from_str(toml).unwrap();
        assert_eq!(config.server.port, 50051);
        assert_eq!(config.server.bind_address, "0.0.0.0");
        assert_eq!(config.security.enrollment_token_ttl, 300);
    }

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
            [server]
            port = 8080
            bind_address = "127.0.0.1"
            mdns_service_name = "test-handcontrol"
            mdns_instance_name = "Test Server"

            [security]
            cert_path = "~/test.crt"
            key_path = "~/test.key"
            authorized_clients_dir = "~/clients"
            enrollment_token_ttl = 600

            [security.enrollment]
            qr_code_enabled = true
            approval_enabled = true
            approval_timeout_seconds = 120
            approval_notification = true

            [[command]]
            id = "test-cmd"
            name = "Test Command"
            description = "A test command"
            shell = "echo {msg}"
            tags = ["test"]
            timeout_seconds = 10

            [[command.parameters]]
            name = "msg"
            type = "text"
            description = "Message to echo"
        "#;

        let config = load_config_from_str(toml).unwrap();
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.bind_address, "127.0.0.1");
        assert_eq!(config.command.len(), 1);
        assert_eq!(config.command[0].id, "test-cmd");
        assert_eq!(config.command[0].parameters.len(), 1);
    }

    #[test]
    fn test_command_defaults() {
        let toml = r#"
            [server]
            [security]

            [[command]]
            id = "minimal"
            name = "Minimal"
            shell = "ls"
        "#;

        let config = load_config_from_str(toml).unwrap();
        assert_eq!(config.command[0].timeout_seconds, 30);
        assert_eq!(config.command[0].tags.len(), 0);
        assert_eq!(config.command[0].parameters.len(), 0);
    }
}
