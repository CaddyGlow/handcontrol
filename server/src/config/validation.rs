use super::parser::{
    CapabilityConfig, CapabilityDefinition, CapabilityParameterConfig, CapabilitySessionMode,
    Config,
};
use anyhow::{Result, bail};
use regex::Regex;
use std::collections::HashSet;

/// Validate the entire configuration
pub fn validate_config(config: &Config) -> Result<()> {
    validate_server_config(config)?;
    validate_capabilities(config)?;
    Ok(())
}

/// Validate server configuration
fn validate_server_config(config: &Config) -> Result<()> {
    if config.server.port == 0 {
        bail!("Server port cannot be 0");
    }

    if config.server.bind_address.is_empty() {
        bail!("Server bind_address cannot be empty");
    }

    if config.server.mdns_service_name.is_empty() {
        bail!("Server mdns_service_name cannot be empty");
    }

    if config.security.enrollment_token_ttl == 0 {
        bail!("Enrollment token TTL cannot be 0");
    }

    if let Some(fingerprint) = config.relay.pinned_cert_sha256.as_ref() {
        if !config.relay.allow_self_signed_tls {
            bail!("relay.pinned_cert_sha256 requires relay.allow_self_signed_tls to be true");
        }

        let normalized: String = fingerprint
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != ':')
            .collect();

        if normalized.len() != 64 || !normalized.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!(
                "relay.pinned_cert_sha256 must be a SHA-256 fingerprint (64 hex characters, colons optional)"
            );
        }
    }

    let relay_url = config
        .relay
        .relay_server_url
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    let relay_secret_missing = config
        .relay
        .relay_auth_secret
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true);

    if config.relay.enabled {
        if relay_url.is_none() {
            bail!("Relay enabled but relay_server_url is missing");
        }

        if relay_secret_missing {
            bail!("Relay enabled but relay_auth_secret is missing");
        }

        if config.relay.reconnect_delay_seconds == 0 {
            bail!("relay.reconnect_delay_seconds must be greater than 0");
        }

        if let Some(max) = config.relay.max_relay_tunnels {
            if max == 0 {
                bail!("relay.max_relay_tunnels must be greater than 0 when specified");
            }
        }
    }

    if config.relay.include_in_enrollment {
        if !config.relay.enabled {
            bail!("relay.include_in_enrollment requires relay.enabled = true");
        }

        if relay_url.is_none() {
            bail!("relay.include_in_enrollment requires relay.relay_server_url to be set");
        }

        if relay_secret_missing {
            bail!("relay.include_in_enrollment requires relay.relay_auth_secret to be configured");
        }

        if config.relay.websocket_subprotocol.trim().is_empty() {
            bail!(
                "relay.websocket_subprotocol cannot be empty when relay.include_in_enrollment is true"
            );
        }
    }

    Ok(())
}

/// Validate capability definitions
fn validate_capabilities(config: &Config) -> Result<()> {
    let mut seen_ids = HashSet::new();
    for capability in &config.capabilities {
        if !seen_ids.insert(&capability.id) {
            bail!("Duplicate capability ID: {}", capability.id);
        }
        validate_capability(capability)?;
    }
    Ok(())
}

fn validate_capability(capability: &CapabilityConfig) -> Result<()> {
    if capability.id.is_empty() {
        bail!("Capability ID cannot be empty");
    }

    if capability.name.is_empty() {
        bail!("Capability '{}' has empty name", capability.id);
    }

    // Ensure parameter names are unique
    let mut seen_params = HashSet::new();
    for param in &capability.parameters {
        if !seen_params.insert(&param.name) {
            bail!(
                "Capability '{}' has duplicate parameter '{}'",
                capability.id,
                param.name
            );
        }
        validate_parameter(&capability.id, param)?;
    }

    match &capability.definition {
        CapabilityDefinition::ShellScript(def) => {
            if def.command.trim().is_empty() {
                bail!(
                    "Capability '{}' shell command cannot be empty",
                    capability.id
                );
            }
            if def.timeout_seconds == 0 {
                bail!("Capability '{}' timeout cannot be 0", capability.id);
            }

            validate_parameter_placeholders(&capability.id, &def.command, &capability.parameters)?;
        }
        CapabilityDefinition::ShellInteractive(def) => {
            if def.shell.trim().is_empty() {
                bail!(
                    "Capability '{}' interactive shell cannot be empty",
                    capability.id
                );
            }

            if let Some(idle) = def.idle_timeout_seconds {
                if idle == 0 {
                    bail!(
                        "Capability '{}' idle_timeout_seconds must be greater than 0",
                        capability.id
                    );
                }
            }

            if let Some(max_duration) = def.max_duration_seconds {
                if max_duration == 0 {
                    bail!(
                        "Capability '{}' max_duration_seconds must be greater than 0",
                        capability.id
                    );
                }
            }

            match def.session_mode.unwrap_or(CapabilitySessionMode::Realtime) {
                CapabilitySessionMode::Realtime => {} // ok
                other => bail!(
                    "Capability '{}' interactive shell must use realtime session mode, found {:?}",
                    capability.id,
                    other
                ),
            }
        }
    }

    Ok(())
}

fn validate_parameter(capability_id: &str, param: &CapabilityParameterConfig) -> Result<()> {
    if param.name.is_empty() {
        bail!(
            "Capability '{}' has parameter with empty name",
            capability_id
        );
    }

    match param.param_type.as_str() {
        "slider" => {
            if param.min.is_none() || param.max.is_none() {
                bail!(
                    "Capability '{}' parameter '{}': slider must define min and max",
                    capability_id,
                    param.name
                );
            }
            let min = param.min.unwrap();
            let max = param.max.unwrap();
            if min >= max {
                bail!(
                    "Capability '{}' parameter '{}': min ({}) must be less than max ({})",
                    capability_id,
                    param.name,
                    min,
                    max
                );
            }
        }
        "text" => {
            if let Some(regex) = &param.validation {
                Regex::new(regex).map_err(|e| {
                    anyhow::anyhow!(
                        "Capability '{}' parameter '{}': invalid validation regex: {}",
                        capability_id,
                        param.name,
                        e
                    )
                })?;
            }
        }
        "toggle" => {}
        "dropdown" => {
            if param.options.is_empty() {
                bail!(
                    "Capability '{}' parameter '{}': dropdown must include options",
                    capability_id,
                    param.name
                );
            }
        }
        other => {
            bail!(
                "Capability '{}' parameter '{}': unsupported type '{}'",
                capability_id,
                param.name,
                other
            );
        }
    }

    Ok(())
}

fn validate_parameter_placeholders(
    capability_id: &str,
    command: &str,
    parameters: &[CapabilityParameterConfig],
) -> Result<()> {
    let param_names: HashSet<_> = parameters.iter().map(|p| p.name.as_str()).collect();
    let placeholder_regex = Regex::new(r"\{(\w+)\}").unwrap();

    for cap in placeholder_regex.captures_iter(command) {
        let placeholder = cap.get(1).unwrap().as_str();
        if !param_names.contains(placeholder) {
            bail!(
                "Capability '{}' shell command references undefined parameter: {}",
                capability_id,
                placeholder
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parser::load_config_from_str;

    #[test]
    fn test_valid_shell_script() {
        let toml = r#"
            [server]
            [security]

            [[capabilities]]
            id = "test"
            name = "Test"
            kind = "shell_script"
            command = "echo {msg}"

            [[capabilities.parameters]]
            name = "msg"
            type = "text"
        "#;

        let config = load_config_from_str(toml).unwrap();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_duplicate_capability_id() {
        let toml = r#"
            [server]
            [security]

            [[capabilities]]
            id = "dup"
            name = "One"
            kind = "shell_script"
            command = "echo one"

            [[capabilities]]
            id = "dup"
            name = "Two"
            kind = "shell_script"
            command = "echo two"
        "#;

        let config = load_config_from_str(toml).unwrap();
        let result = validate_config(&config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Duplicate capability ID")
        );
    }

    #[test]
    fn test_undefined_placeholder() {
        let toml = r#"
            [server]
            [security]

            [[capabilities]]
            id = "test"
            name = "Test"
            kind = "shell_script"
            command = "echo {missing}"
        "#;

        let config = load_config_from_str(toml).unwrap();
        let result = validate_config(&config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("undefined parameter")
        );
    }

    #[test]
    fn test_invalid_slider_bounds() {
        let toml = r#"
            [server]
            [security]

            [[capabilities]]
            id = "test"
            name = "Test"
            kind = "shell_script"
            command = "echo {val}"

            [[capabilities.parameters]]
            name = "val"
            type = "slider"
            min = 100
            max = 10
        "#;

        let config = load_config_from_str(toml).unwrap();
        let result = validate_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("min (100)"));
    }

    #[test]
    fn test_interactive_shell_requires_realtime() {
        let toml = r#"
            [server]
            [security]

            [[capabilities]]
            id = "interactive"
            name = "Interactive"
            kind = "shell_interactive"
            session_mode = "one_shot"
        "#;

        let config = load_config_from_str(toml).unwrap();
        let result = validate_config(&config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("interactive shell must use realtime session mode")
        );
    }
}
