use super::parser::{CommandConfig, Config, ParameterConfig};
use anyhow::{bail, Result};
use regex::Regex;
use std::collections::HashSet;

/// Validate the entire configuration
pub fn validate_config(config: &Config) -> Result<()> {
    validate_server_config(config)?;
    validate_commands(config)?;
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

    Ok(())
}

/// Validate all command definitions
fn validate_commands(config: &Config) -> Result<()> {
    // Check for duplicate command IDs
    let mut seen_ids = HashSet::new();
    for cmd in &config.command {
        if !seen_ids.insert(&cmd.id) {
            bail!("Duplicate command ID: {}", cmd.id);
        }
    }

    // Validate each command
    for cmd in &config.command {
        validate_command(cmd)?;
    }

    Ok(())
}

/// Validate a single command
fn validate_command(cmd: &CommandConfig) -> Result<()> {
    if cmd.id.is_empty() {
        bail!("Command ID cannot be empty");
    }

    if cmd.name.is_empty() {
        bail!("Command '{}' has empty name", cmd.id);
    }

    if cmd.shell.is_empty() {
        bail!("Command '{}' has empty shell command", cmd.id);
    }

    if cmd.timeout_seconds == 0 {
        bail!("Command '{}' timeout cannot be 0", cmd.id);
    }

    // Check for duplicate parameter names
    let mut seen_params = HashSet::new();
    for param in &cmd.parameters {
        if !seen_params.insert(&param.name) {
            bail!(
                "Command '{}' has duplicate parameter: {}",
                cmd.id,
                param.name
            );
        }
    }

    // Validate each parameter
    for param in &cmd.parameters {
        validate_parameter(&cmd.id, param)?;
    }

    // Check that all parameter placeholders in shell command are defined
    validate_parameter_placeholders(cmd)?;

    Ok(())
}

/// Validate a single parameter
fn validate_parameter(cmd_id: &str, param: &ParameterConfig) -> Result<()> {
    if param.name.is_empty() {
        bail!("Command '{}' has parameter with empty name", cmd_id);
    }

    match param.param_type.as_str() {
        "slider" => {
            if param.min.is_none() || param.max.is_none() {
                bail!(
                    "Command '{}' parameter '{}': slider must have min and max",
                    cmd_id,
                    param.name
                );
            }
            let min = param.min.unwrap();
            let max = param.max.unwrap();
            if min >= max {
                bail!(
                    "Command '{}' parameter '{}': min ({}) must be less than max ({})",
                    cmd_id,
                    param.name,
                    min,
                    max
                );
            }
        }
        "text" => {
            // Validate regex if provided
            if let Some(ref regex) = param.validation {
                Regex::new(regex).map_err(|e| {
                    anyhow::anyhow!(
                        "Command '{}' parameter '{}': invalid validation regex: {}",
                        cmd_id,
                        param.name,
                        e
                    )
                })?;
            }
        }
        "toggle" => {
            // No specific validation needed
        }
        "dropdown" => {
            if param.options.is_empty() {
                bail!(
                    "Command '{}' parameter '{}': dropdown must have options",
                    cmd_id,
                    param.name
                );
            }
        }
        _ => {
            bail!(
                "Command '{}' parameter '{}': invalid type '{}'",
                cmd_id,
                param.name,
                param.param_type
            );
        }
    }

    Ok(())
}

/// Validate that all parameter placeholders in shell command are defined
fn validate_parameter_placeholders(cmd: &CommandConfig) -> Result<()> {
    let param_names: HashSet<_> = cmd.parameters.iter().map(|p| &p.name).collect();

    // Find all {param} placeholders in shell command
    let placeholder_regex = Regex::new(r"\{(\w+)\}").unwrap();
    for cap in placeholder_regex.captures_iter(&cmd.shell) {
        let placeholder = &cap[1];
        if !param_names.contains(&placeholder.to_string()) {
            bail!(
                "Command '{}' shell command references undefined parameter: {}",
                cmd.id,
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
    fn test_valid_config() {
        let toml = r#"
            [server]
            port = 50051

            [security]

            [[command]]
            id = "test"
            name = "Test"
            shell = "echo {msg}"

            [[command.parameters]]
            name = "msg"
            type = "text"
        "#;

        let config = load_config_from_str(toml).unwrap();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_duplicate_command_id() {
        let toml = r#"
            [server]
            [security]

            [[command]]
            id = "test"
            name = "Test 1"
            shell = "echo 1"

            [[command]]
            id = "test"
            name = "Test 2"
            shell = "echo 2"
        "#;

        let config = load_config_from_str(toml).unwrap();
        let result = validate_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Duplicate"));
    }

    #[test]
    fn test_undefined_parameter() {
        let toml = r#"
            [server]
            [security]

            [[command]]
            id = "test"
            name = "Test"
            shell = "echo {undefined}"
        "#;

        let config = load_config_from_str(toml).unwrap();
        let result = validate_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("undefined"));
    }

    #[test]
    fn test_invalid_slider() {
        let toml = r#"
            [server]
            [security]

            [[command]]
            id = "test"
            name = "Test"
            shell = "echo {val}"

            [[command.parameters]]
            name = "val"
            type = "slider"
            min = 100
            max = 10
        "#;

        let config = load_config_from_str(toml).unwrap();
        let result = validate_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("min"));
    }

    #[test]
    fn test_empty_dropdown_options() {
        let toml = r#"
            [server]
            [security]

            [[command]]
            id = "test"
            name = "Test"
            shell = "echo {choice}"

            [[command.parameters]]
            name = "choice"
            type = "dropdown"
        "#;

        let config = load_config_from_str(toml).unwrap();
        let result = validate_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("options"));
    }
}
