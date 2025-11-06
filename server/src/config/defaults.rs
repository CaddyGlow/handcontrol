use super::parser::{
    CapabilityAclConfig, CapabilityConfig, CapabilityDefinition, CapabilityParameterConfig,
    CapabilitySessionMode, Config, EnrollmentConfig, NetworkConfig, RelayConfig, SecurityConfig,
    ServerConfig, ShellInteractiveDefinition, ShellScriptDefinition,
};
use anyhow::Result;
use std::collections::HashMap;

/// Generate a default configuration with example commands for the current platform
pub fn generate_default_config() -> Result<Config> {
    let capabilities = generate_platform_capabilities();

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
            require_client_cert: false,
            enrollment: EnrollmentConfig {
                qr_code_enabled: true,
                approval_enabled: true,
                approval_timeout_seconds: 60,
                approval_notification: true,
            },
        },
        network: NetworkConfig::default(),
        relay: RelayConfig::default(),
        capabilities,
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
fn generate_platform_capabilities() -> Vec<CapabilityConfig> {
    vec![
        CapabilityConfig {
            id: "lock-screen".to_string(),
            name: "Lock Screen".to_string(),
            description: Some("Locks the computer screen".to_string()),
            tags: vec!["system".to_string(), "security".to_string()],
            requires_confirmation: false,
            privileged: false,
            parameters: vec![],
            acl: CapabilityAclConfig::default(),
            definition: CapabilityDefinition::ShellScript(ShellScriptDefinition {
                command: "loginctl lock-session".to_string(),
                timeout_seconds: 5,
                env: HashMap::new(),
                show_output: true,
                session_mode: CapabilitySessionMode::OneShot,
            }),
        },
        CapabilityConfig {
            id: "suspend".to_string(),
            name: "Suspend".to_string(),
            description: Some("Suspends the computer".to_string()),
            tags: vec!["system".to_string(), "power".to_string()],
            requires_confirmation: true,
            privileged: false,
            parameters: vec![],
            acl: CapabilityAclConfig::default(),
            definition: CapabilityDefinition::ShellScript(ShellScriptDefinition {
                command: "systemctl suspend".to_string(),
                timeout_seconds: 5,
                env: HashMap::new(),
                show_output: true,
                session_mode: CapabilitySessionMode::OneShot,
            }),
        },
        CapabilityConfig {
            id: "set-volume".to_string(),
            name: "Set Volume".to_string(),
            description: Some("Adjust system volume".to_string()),
            tags: vec!["media".to_string(), "audio".to_string()],
            requires_confirmation: false,
            privileged: false,
            parameters: vec![CapabilityParameterConfig {
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
                default_value_command: None,
                default_value_pattern: None,
            }],
            acl: CapabilityAclConfig::default(),
            definition: CapabilityDefinition::ShellScript(ShellScriptDefinition {
                command: "pactl set-sink-volume @DEFAULT_SINK@ {level}%".to_string(),
                timeout_seconds: 3,
                env: HashMap::new(),
                show_output: false,
                session_mode: CapabilitySessionMode::OneShot,
            }),
        },
        CapabilityConfig {
            id: "toggle-mute".to_string(),
            name: "Toggle Mute".to_string(),
            description: Some("Mute or unmute audio".to_string()),
            tags: vec!["media".to_string(), "audio".to_string()],
            requires_confirmation: false,
            privileged: false,
            parameters: vec![CapabilityParameterConfig {
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
                default_value_command: None,
                default_value_pattern: None,
            }],
            acl: CapabilityAclConfig::default(),
            definition: CapabilityDefinition::ShellScript(ShellScriptDefinition {
                command: "pactl set-sink-mute @DEFAULT_SINK@ {muted}".to_string(),
                timeout_seconds: 3,
                env: HashMap::new(),
                show_output: false,
                session_mode: CapabilitySessionMode::OneShot,
            }),
        },
        default_interactive_shell_capability(),
    ]
}

#[cfg(target_os = "windows")]
fn generate_platform_capabilities() -> Vec<CapabilityConfig> {
    vec![
        CapabilityConfig {
            id: "lock-screen".to_string(),
            name: "Lock Screen".to_string(),
            description: Some("Locks the computer screen".to_string()),
            tags: vec!["system".to_string(), "security".to_string()],
            requires_confirmation: false,
            privileged: false,
            parameters: vec![],
            acl: CapabilityAclConfig::default(),
            definition: CapabilityDefinition::ShellScript(ShellScriptDefinition {
                command: "rundll32.exe user32.dll,LockWorkStation".to_string(),
                timeout_seconds: 5,
                env: HashMap::new(),
                show_output: true,
                session_mode: CapabilitySessionMode::OneShot,
            }),
        },
        CapabilityConfig {
            id: "shutdown".to_string(),
            name: "Shutdown".to_string(),
            description: Some("Shuts down the computer".to_string()),
            tags: vec!["system".to_string(), "power".to_string()],
            requires_confirmation: true,
            privileged: false,
            parameters: vec![],
            acl: CapabilityAclConfig::default(),
            definition: CapabilityDefinition::ShellScript(ShellScriptDefinition {
                command: "shutdown /s /t 0".to_string(),
                timeout_seconds: 5,
                env: HashMap::new(),
                show_output: true,
                session_mode: CapabilitySessionMode::OneShot,
            }),
        },
        default_interactive_shell_capability(),
    ]
}

#[cfg(target_os = "macos")]
fn generate_platform_capabilities() -> Vec<CapabilityConfig> {
    vec![
        CapabilityConfig {
            id: "lock-screen".to_string(),
            name: "Lock Screen".to_string(),
            description: Some("Locks the computer screen".to_string()),
            tags: vec!["system".to_string(), "security".to_string()],
            requires_confirmation: false,
            privileged: false,
            parameters: vec![],
            acl: CapabilityAclConfig::default(),
            definition: CapabilityDefinition::ShellScript(ShellScriptDefinition {
                command: "/System/Library/CoreServices/Menu\\ Extras/User.menu/Contents/Resources/CGSession -suspend".to_string(),
                timeout_seconds: 5,
                env: HashMap::new(),
                show_output: true,
                session_mode: CapabilitySessionMode::OneShot,
            }),
        },
        CapabilityConfig {
            id: "set-volume".to_string(),
            name: "Set Volume".to_string(),
            description: Some("Adjust system volume".to_string()),
            tags: vec!["media".to_string(), "audio".to_string()],
            requires_confirmation: false,
            privileged: false,
            parameters: vec![CapabilityParameterConfig {
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
                default_value_command: None,
                default_value_pattern: None,
            }],
            acl: CapabilityAclConfig::default(),
            definition: CapabilityDefinition::ShellScript(ShellScriptDefinition {
                command: "osascript -e 'set volume output volume {level}'".to_string(),
                timeout_seconds: 3,
                env: HashMap::new(),
                show_output: false,
                session_mode: CapabilitySessionMode::OneShot,
            }),
        },
        default_interactive_shell_capability(),
    ]
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn generate_platform_capabilities() -> Vec<CapabilityConfig> {
    vec![default_interactive_shell_capability()]
}

fn default_interactive_shell_capability() -> CapabilityConfig {
    CapabilityConfig {
        id: "shell.interactive".to_string(),
        name: "Interactive Shell".to_string(),
        description: Some("Open an interactive shell session".to_string()),
        tags: vec!["system".to_string(), "shell".to_string()],
        requires_confirmation: true,
        privileged: true,
        parameters: vec![],
        acl: CapabilityAclConfig::default(),
        definition: CapabilityDefinition::ShellInteractive(ShellInteractiveDefinition {
            path: if cfg!(target_os = "windows") {
                "cmd.exe".to_string()
            } else {
                "/bin/sh".to_string()
            },
            argv: vec!["-i".to_string()],
            working_directory: None,
            env: HashMap::new(),
            idle_timeout_seconds: Some(600),
            max_duration_seconds: Some(3600),
            session_mode: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_default_config() {
        let config = generate_default_config().unwrap();
        assert_eq!(config.server.port, 50051);
        assert!(!config.capabilities.is_empty());
    }

    #[test]
    fn test_generate_default_config_toml() {
        let toml = generate_default_config_toml().unwrap();
        assert!(toml.contains("[server]"));
        assert!(toml.contains("[security]"));
        assert!(toml.contains("[[capabilities]]"));
    }
}
