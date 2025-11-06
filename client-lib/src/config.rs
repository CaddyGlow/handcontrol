use anyhow::{anyhow, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fmt, fs, path::PathBuf, str::FromStr};

const QUALIFIER: &str = "";
const ORGANIZATION: &str = "";
const APPLICATION: &str = "handcontrol";
const CONFIG_FILE_NAME: &str = "client.toml";
pub const TRANSPORT_OVERRIDE_ENV: &str = "HANDCONTROL_CLIENT_TRANSPORT";
const CONFIG_DIR_ENV_VAR: &str = "HANDCONTROL_CONFIG_DIR";

/// Top-level client configuration loaded from `client.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    #[serde(default)]
    pub discovery: DiscoveryConfig,

    #[serde(default)]
    pub connection: ConnectionConfig,

    #[serde(default)]
    pub network: NetworkConfig,

    #[serde(default)]
    pub tui: TuiConfig,

    #[serde(default)]
    pub cli: CliConfig,

    #[serde(default)]
    pub device: Option<DeviceConfig>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            discovery: DiscoveryConfig::default(),
            connection: ConnectionConfig::default(),
            network: NetworkConfig::default(),
            tui: TuiConfig::default(),
            cli: CliConfig::default(),
            device: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default)]
    pub relay: RelayBehaviorConfig,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            relay: RelayBehaviorConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayBehaviorConfig {
    #[serde(default)]
    pub prefer_relay: bool,
    #[serde(default)]
    pub relay_only_mode: bool,
    #[serde(default = "default_max_direct_attempts")]
    pub max_direct_attempts: u32,
    #[serde(default)]
    pub transport: TransportPreference,
}

impl Default for RelayBehaviorConfig {
    fn default() -> Self {
        Self {
            prefer_relay: false,
            relay_only_mode: false,
            max_direct_attempts: default_max_direct_attempts(),
            transport: TransportPreference::default(),
        }
    }
}

/// Discovery-related settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    #[serde(default = "default_true")]
    pub auto_discover: bool,
    #[serde(default = "default_discovery_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "default_true")]
    pub prefer_ipv6: bool,
    #[serde(default)]
    pub include_link_local: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            auto_discover: true,
            timeout_seconds: 5,
            prefer_ipv6: true,
            include_link_local: false,
        }
    }
}

/// Connection timeout & retry behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_command_timeout")]
    pub command_timeout_seconds: u64,
    #[serde(default = "default_retry_attempts")]
    pub retry_attempts: u32,
    #[serde(default = "default_retry_delay")]
    pub retry_delay_ms: u64,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 5,
            command_timeout_seconds: 300,
            retry_attempts: 3,
            retry_delay_ms: 1_000,
        }
    }
}

/// Visual and behavioral options for the TUI client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiConfig {
    #[serde(default = "default_true")]
    pub show_timestamps: bool,
    #[serde(default = "default_color_scheme")]
    pub color_scheme: String,
    #[serde(default = "default_true")]
    pub auto_scroll: bool,
    #[serde(default)]
    pub confirm_commands: bool,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            show_timestamps: true,
            color_scheme: "default".to_string(),
            auto_scroll: true,
            confirm_commands: false,
        }
    }
}

/// Output formatting for CLI workflows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConfig {
    #[serde(default = "default_output_format")]
    pub output_format: String,
    #[serde(default = "default_true")]
    pub show_headers: bool,
    #[serde(default = "default_color_output")]
    pub color_output: String,
    #[serde(default = "default_true")]
    pub jiggle_resize_on_resume: bool,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            output_format: "tsv".to_string(),
            show_headers: true,
            color_output: "auto".to_string(),
            jiggle_resize_on_resume: true,
        }
    }
}

/// Optional device metadata stored with enrollments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub name: Option<String>,
    pub model: Option<String>,
}

pub fn config_dir() -> Result<PathBuf> {
    if let Some(custom) = std::env::var_os(CONFIG_DIR_ENV_VAR) {
        let dir = PathBuf::from(custom);
        if dir.as_os_str().is_empty() {
            return Err(anyhow!(
                "{CONFIG_DIR_ENV_VAR} is set but empty; provide a valid directory path"
            ));
        }
        fs::create_dir_all(&dir).with_context(|| {
            format!(
                "Failed to create config directory {} from {CONFIG_DIR_ENV_VAR}",
                dir.display()
            )
        })?;
        return Ok(dir);
    }

    let project_dirs = ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .ok_or_else(|| anyhow!("Unable to determine configuration directory"))?;

    let config_dir = project_dirs.config_dir().to_path_buf();
    fs::create_dir_all(&config_dir)
        .with_context(|| format!("Failed to create config directory {}", config_dir.display()))?;
    Ok(config_dir)
}

pub fn config_path() -> Result<PathBuf> {
    let mut path = config_dir()?;
    path.push(CONFIG_FILE_NAME);
    Ok(path)
}

/// Load configuration from disk, returning defaults when the file is missing or empty.
pub fn load() -> Result<ClientConfig> {
    let path = config_path()?;

    if !path.exists() {
        let mut cfg = ClientConfig::default();
        apply_env_overrides(&mut cfg);
        return Ok(cfg);
    }

    let contents =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;

    if contents.trim().is_empty() {
        let mut cfg = ClientConfig::default();
        apply_env_overrides(&mut cfg);
        return Ok(cfg);
    }

    let mut cfg: ClientConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse client config at {}", path.display()))?;
    apply_env_overrides(&mut cfg);
    Ok(cfg)
}

/// Persist configuration to disk, creating parent directories as needed.
pub fn save(config: &ClientConfig) -> Result<()> {
    let path = config_path()?;
    let contents = toml::to_string_pretty(config).context("Failed to serialize client config")?;
    fs::write(&path, contents).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

fn default_true() -> bool {
    true
}

fn default_discovery_timeout() -> u64 {
    5
}

fn default_timeout_seconds() -> u64 {
    5
}

fn default_command_timeout() -> u64 {
    300
}

fn default_max_direct_attempts() -> u32 {
    3
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TransportPreference {
    Auto,
    Websocket,
    Quic,
}

impl Default for TransportPreference {
    fn default() -> Self {
        TransportPreference::Auto
    }
}

impl fmt::Display for TransportPreference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TransportPreference::Auto => "auto",
            TransportPreference::Websocket => "websocket",
            TransportPreference::Quic => "quic",
        })
    }
}

impl FromStr for TransportPreference {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "auto" => Ok(TransportPreference::Auto),
            "websocket" | "ws" => Ok(TransportPreference::Websocket),
            "quic" => Ok(TransportPreference::Quic),
            other => Err(anyhow::anyhow!(
                "Unsupported transport preference '{other}'"
            )),
        }
    }
}

fn apply_env_overrides(config: &mut ClientConfig) {
    if let Ok(value) = std::env::var(TRANSPORT_OVERRIDE_ENV) {
        match value.parse::<TransportPreference>() {
            Ok(pref) => config.network.relay.transport = pref,
            Err(err) => {
                tracing::warn!("Ignoring transport override '{}': {:#}", value, err);
            }
        }
    }
}

fn default_retry_attempts() -> u32 {
    3
}

fn default_retry_delay() -> u64 {
    1_000
}

fn default_color_scheme() -> String {
    "default".to_string()
}

fn default_output_format() -> String {
    "tsv".to_string()
}

fn default_color_output() -> String {
    "auto".to_string()
}
