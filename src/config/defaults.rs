use super::parser::{
    CommandConfig, Config, EnrollmentConfig, ParameterConfig, SecurityConfig, ServerConfig,
};
use anyhow::Result;
use std::collections::HashMap;

/// Generate a default configuration with example commands for the current platform
pub fn generate_default_config() -> Result<Config> {
    let commands = generate_platform_commands();

    Ok(Config {
        server: ServerConfig {
            port: 50051,
            bind_address: "::".to_string(),
            mdns_service_name: "handcontrol".to_string(),
            mdns_instance_name: None,
        },
        security: SecurityConfig {
            cert_path: None,
            key_path: None,
            authorized_clients_dir: None,
            enrollment_token_ttl: 300,
            enrollment: EnrollmentConfig {
                qr_code_enabled: true,
                approval_enabled: true,
                approval_timeout_seconds: 60,
                approval_notification: true,
            },
        },
        command: commands,
    })
}

/// Generate default TOML configuration as a string
pub fn generate_default_config_toml() -> Result<String> {
    let config = generate_default_config()?;
    let toml_string = toml::to_string_pretty(&config)?;
    Ok(toml_string)
}

/// Generate platform-specific example commands
#[cfg(target_os = "linux")]
fn generate_platform_commands() -> Vec<CommandConfig> {
    vec![
        CommandConfig {
            id: "lock-screen".to_string(),
            name: "Lock Screen".to_string(),
            description: Some("Locks the computer screen".to_string()),
            icon: Some("lock".to_string()),
            shell: "loginctl lock-session".to_string(),
            tags: vec!["system".to_string(), "security".to_string()],
            timeout_seconds: 5,
            env: HashMap::new(),
            parameters: vec![],
        },
        CommandConfig {
            id: "suspend".to_string(),
            name: "Suspend".to_string(),
            description: Some("Suspends the computer".to_string()),
            icon: Some("power".to_string()),
            shell: "systemctl suspend".to_string(),
            tags: vec!["system".to_string(), "power".to_string()],
            timeout_seconds: 5,
            env: HashMap::new(),
            parameters: vec![],
        },
        CommandConfig {
            id: "set-volume".to_string(),
            name: "Set Volume".to_string(),
            description: Some("Adjust system volume".to_string()),
            icon: Some("volume".to_string()),
            shell: "pactl set-sink-volume @DEFAULT_SINK@ {level}%".to_string(),
            tags: vec!["media".to_string(), "audio".to_string()],
            timeout_seconds: 3,
            env: HashMap::new(),
            parameters: vec![ParameterConfig {
                name: "level".to_string(),
                param_type: "slider".to_string(),
                description: Some("Volume level (0-100)".to_string()),
                min: Some(0),
                max: Some(100),
                default: Some("50".to_string()),
                options: vec![],
                validation: None,
                label_on: None,
                label_off: None,
                step: Some(5),
            }],
        },
        CommandConfig {
            id: "toggle-mute".to_string(),
            name: "Toggle Mute".to_string(),
            description: Some("Mute or unmute audio".to_string()),
            icon: Some("volume-mute".to_string()),
            shell: "pactl set-sink-mute @DEFAULT_SINK@ {muted}".to_string(),
            tags: vec!["media".to_string(), "audio".to_string()],
            timeout_seconds: 3,
            env: HashMap::new(),
            parameters: vec![ParameterConfig {
                name: "muted".to_string(),
                param_type: "toggle".to_string(),
                description: Some("Mute state".to_string()),
                min: None,
                max: None,
                default: Some("1".to_string()),
                options: vec![],
                validation: None,
                label_on: Some("Mute".to_string()),
                label_off: Some("Unmute".to_string()),
                step: None,
            }],
        },
    ]
}

#[cfg(target_os = "windows")]
fn generate_platform_commands() -> Vec<CommandConfig> {
    vec![
        CommandConfig {
            id: "lock-screen".to_string(),
            name: "Lock Screen".to_string(),
            description: Some("Locks the computer screen".to_string()),
            icon: Some("lock".to_string()),
            shell: "rundll32.exe user32.dll,LockWorkStation".to_string(),
            tags: vec!["system".to_string(), "security".to_string()],
            timeout_seconds: 5,
            env: HashMap::new(),
            parameters: vec![],
        },
        CommandConfig {
            id: "shutdown".to_string(),
            name: "Shutdown".to_string(),
            description: Some("Shuts down the computer".to_string()),
            icon: Some("power".to_string()),
            shell: "shutdown /s /t 0".to_string(),
            tags: vec!["system".to_string(), "power".to_string()],
            timeout_seconds: 5,
            env: HashMap::new(),
            parameters: vec![],
        },
    ]
}

#[cfg(target_os = "macos")]
fn generate_platform_commands() -> Vec<CommandConfig> {
    vec![
        CommandConfig {
            id: "lock-screen".to_string(),
            name: "Lock Screen".to_string(),
            description: Some("Locks the computer screen".to_string()),
            icon: Some("lock".to_string()),
            shell: "/System/Library/CoreServices/Menu\\ Extras/User.menu/Contents/Resources/CGSession -suspend".to_string(),
            tags: vec!["system".to_string(), "security".to_string()],
            timeout_seconds: 5,
            env: HashMap::new(),
            parameters: vec![],
        },
        CommandConfig {
            id: "set-volume".to_string(),
            name: "Set Volume".to_string(),
            description: Some("Adjust system volume".to_string()),
            icon: Some("volume".to_string()),
            shell: "osascript -e 'set volume output volume {level}'".to_string(),
            tags: vec!["media".to_string(), "audio".to_string()],
            timeout_seconds: 3,
            env: HashMap::new(),
            parameters: vec![ParameterConfig {
                name: "level".to_string(),
                param_type: "slider".to_string(),
                description: Some("Volume level (0-100)".to_string()),
                min: Some(0),
                max: Some(100),
                default: Some("50".to_string()),
                options: vec![],
                validation: None,
                label_on: None,
                label_off: None,
                step: Some(5),
            }],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_default_config() {
        let config = generate_default_config().unwrap();
        assert_eq!(config.server.port, 50051);
        assert!(!config.command.is_empty());
    }

    #[test]
    fn test_generate_default_config_toml() {
        let toml = generate_default_config_toml().unwrap();
        assert!(toml.contains("[server]"));
        assert!(toml.contains("[security]"));
        assert!(toml.contains("[[command]]"));
    }
}
