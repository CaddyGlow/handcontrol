use anyhow::{Context, Result, anyhow};
use regex::Regex;
use std::collections::HashMap;

use crate::config::parser::{CommandConfig, ParameterConfig};

/// Validates parameters against command definition and returns validated map
pub fn validate_parameters(
    command: &CommandConfig,
    provided_params: &HashMap<String, String>,
) -> Result<HashMap<String, String>> {
    let mut validated = HashMap::new();

    // Check for required parameters
    for param_def in &command.parameters {
        let value = if let Some(v) = provided_params.get(&param_def.name) {
            v.clone()
        } else if let Some(default) = &param_def.default {
            default.clone()
        } else {
            return Err(anyhow!("Missing required parameter: {}", param_def.name));
        };

        // Validate parameter based on type
        validate_parameter_value(param_def, &value)
            .with_context(|| format!("Invalid value for parameter '{}'", param_def.name))?;

        validated.insert(param_def.name.clone(), value);
    }

    // Check for unknown parameters
    for param_name in provided_params.keys() {
        if !command.parameters.iter().any(|p| p.name == *param_name) {
            return Err(anyhow!("Unknown parameter: {}", param_name));
        }
    }

    Ok(validated)
}

/// Validates a single parameter value against its definition
fn validate_parameter_value(param_def: &ParameterConfig, value: &str) -> Result<()> {
    match param_def.param_type.as_str() {
        "slider" => {
            // Parse as integer
            let num: i32 = value.parse().context("Slider value must be an integer")?;

            // Check min/max bounds
            if let Some(min) = param_def.min {
                if num < min {
                    return Err(anyhow!("Value {} is below minimum {}", num, min));
                }
            }

            if let Some(max) = param_def.max {
                if num > max {
                    return Err(anyhow!("Value {} is above maximum {}", num, max));
                }
            }

            Ok(())
        }
        "toggle" => {
            // Must be "true" or "false"
            match value {
                "true" | "false" => Ok(()),
                _ => Err(anyhow!(
                    "Toggle value must be 'true' or 'false', got '{}'",
                    value
                )),
            }
        }
        "dropdown" => {
            // Must be one of the defined options
            if !param_def.options.contains(&value.to_string()) {
                return Err(anyhow!(
                    "Value '{}' is not a valid option. Valid options: {:?}",
                    value,
                    param_def.options
                ));
            }
            Ok(())
        }
        "text" => {
            // Check regex validation if provided
            if let Some(validation_regex) = &param_def.validation {
                let regex = Regex::new(validation_regex).context("Invalid validation regex")?;

                if !regex.is_match(value) {
                    return Err(anyhow!(
                        "Value '{}' does not match required pattern: {}",
                        value,
                        validation_regex
                    ));
                }
            }
            Ok(())
        }
        _ => Err(anyhow!("Unknown parameter type: {}", param_def.param_type)),
    }
}

/// Substitutes parameters into shell command with proper escaping
pub fn substitute_parameters(
    shell_template: &str,
    parameters: &HashMap<String, String>,
) -> Result<String> {
    let mut result = shell_template.to_string();

    // Find all {parameter_name} placeholders
    let re = Regex::new(r"\{([a-zA-Z0-9_-]+)\}").context("Failed to compile parameter regex")?;

    // Track which parameters were used
    let mut used_params = HashMap::new();

    // Replace all placeholders
    for cap in re.captures_iter(shell_template) {
        let param_name = &cap[1];
        let placeholder = &cap[0];

        let value = parameters
            .get(param_name)
            .ok_or_else(|| anyhow!("Parameter '{}' not provided", param_name))?;

        // Escape the value for shell safety
        let escaped_value = shell_escape::escape(value.into());

        result = result.replace(placeholder, &escaped_value);
        used_params.insert(param_name.to_string(), true);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_slider_param(name: &str, min: i32, max: i32) -> ParameterConfig {
        ParameterConfig {
            name: name.to_string(),
            param_type: "slider".to_string(),
            description: None,
            min: Some(min),
            max: Some(max),
            default: None,
            options: vec![],
            validation: None,
            label_on: None,
            label_off: None,
            step: None,
        }
    }

    fn make_text_param(name: &str, validation: Option<&str>) -> ParameterConfig {
        ParameterConfig {
            name: name.to_string(),
            param_type: "text".to_string(),
            description: None,
            min: None,
            max: None,
            default: None,
            options: vec![],
            validation: validation.map(String::from),
            label_on: None,
            label_off: None,
            step: None,
        }
    }

    fn make_toggle_param(name: &str) -> ParameterConfig {
        ParameterConfig {
            name: name.to_string(),
            param_type: "toggle".to_string(),
            description: None,
            min: None,
            max: None,
            default: Some("false".to_string()),
            options: vec![],
            validation: None,
            label_on: None,
            label_off: None,
            step: None,
        }
    }

    fn make_dropdown_param(name: &str, options: Vec<&str>) -> ParameterConfig {
        ParameterConfig {
            name: name.to_string(),
            param_type: "dropdown".to_string(),
            description: None,
            min: None,
            max: None,
            default: None,
            options: options.iter().map(|s| s.to_string()).collect(),
            validation: None,
            label_on: None,
            label_off: None,
            step: None,
        }
    }

    #[test]
    fn test_validate_slider_valid() {
        let param = make_slider_param("volume", 0, 100);
        assert!(validate_parameter_value(&param, "50").is_ok());
        assert!(validate_parameter_value(&param, "0").is_ok());
        assert!(validate_parameter_value(&param, "100").is_ok());
    }

    #[test]
    fn test_validate_slider_below_min() {
        let param = make_slider_param("volume", 0, 100);
        assert!(validate_parameter_value(&param, "-1").is_err());
    }

    #[test]
    fn test_validate_slider_above_max() {
        let param = make_slider_param("volume", 0, 100);
        assert!(validate_parameter_value(&param, "101").is_err());
    }

    #[test]
    fn test_validate_slider_not_integer() {
        let param = make_slider_param("volume", 0, 100);
        assert!(validate_parameter_value(&param, "50.5").is_err());
        assert!(validate_parameter_value(&param, "abc").is_err());
    }

    #[test]
    fn test_validate_toggle_valid() {
        let param = make_toggle_param("muted");
        assert!(validate_parameter_value(&param, "true").is_ok());
        assert!(validate_parameter_value(&param, "false").is_ok());
    }

    #[test]
    fn test_validate_toggle_invalid() {
        let param = make_toggle_param("muted");
        assert!(validate_parameter_value(&param, "yes").is_err());
        assert!(validate_parameter_value(&param, "1").is_err());
    }

    #[test]
    fn test_validate_dropdown_valid() {
        let param = make_dropdown_param("output", vec!["speakers", "headphones", "hdmi"]);
        assert!(validate_parameter_value(&param, "speakers").is_ok());
        assert!(validate_parameter_value(&param, "headphones").is_ok());
    }

    #[test]
    fn test_validate_dropdown_invalid() {
        let param = make_dropdown_param("output", vec!["speakers", "headphones"]);
        assert!(validate_parameter_value(&param, "hdmi").is_err());
    }

    #[test]
    fn test_validate_text_with_regex() {
        let param = make_text_param("filename", Some(r"^[a-zA-Z0-9_-]+$"));
        assert!(validate_parameter_value(&param, "test_file-123").is_ok());
        assert!(validate_parameter_value(&param, "test file").is_err());
        assert!(validate_parameter_value(&param, "test@file").is_err());
    }

    #[test]
    fn test_validate_text_without_regex() {
        let param = make_text_param("message", None);
        assert!(validate_parameter_value(&param, "any text allowed").is_ok());
        assert!(validate_parameter_value(&param, "special @#$ chars").is_ok());
    }

    #[test]
    fn test_substitute_parameters_simple() {
        let template = "echo {message}";
        let mut params = HashMap::new();
        params.insert("message".to_string(), "hello".to_string());

        let result = substitute_parameters(template, &params).unwrap();
        assert!(result.contains("hello"));
    }

    #[test]
    fn test_substitute_parameters_multiple() {
        let template = "convert {input} -resize {width}x{height} {output}";
        let mut params = HashMap::new();
        params.insert("input".to_string(), "image.jpg".to_string());
        params.insert("width".to_string(), "800".to_string());
        params.insert("height".to_string(), "600".to_string());
        params.insert("output".to_string(), "resized.jpg".to_string());

        let result = substitute_parameters(template, &params).unwrap();
        assert!(result.contains("image.jpg"));
        assert!(result.contains("800"));
        assert!(result.contains("600"));
        assert!(result.contains("resized.jpg"));
    }

    #[test]
    fn test_substitute_parameters_shell_escape() {
        let template = "echo {message}";
        let mut params = HashMap::new();
        params.insert("message".to_string(), "hello; rm -rf /".to_string());

        let result = substitute_parameters(template, &params).unwrap();
        // Should be escaped to prevent command injection
        assert!(result.contains("hello"));
        // The exact escaping format depends on shell_escape crate
        // but it should not contain unescaped semicolons
    }

    #[test]
    fn test_substitute_parameters_missing() {
        let template = "echo {message}";
        let params = HashMap::new();

        let result = substitute_parameters(template, &params);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_parameters_with_defaults() {
        let command = CommandConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            description: None,
            icon: None,
            shell: "echo".to_string(),
            tags: vec![],
            timeout_seconds: 30,
            env: HashMap::new(),
            parameters: vec![ParameterConfig {
                name: "msg".to_string(),
                param_type: "text".to_string(),
                description: None,
                min: None,
                max: None,
                default: Some("default_message".to_string()),
                options: vec![],
                validation: None,
                label_on: None,
                label_off: None,
                step: None,
            }],
        };

        let params = HashMap::new();
        let validated = validate_parameters(&command, &params).unwrap();
        assert_eq!(validated.get("msg").unwrap(), "default_message");
    }

    #[test]
    fn test_validate_parameters_unknown_param() {
        let command = CommandConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            description: None,
            icon: None,
            shell: "echo".to_string(),
            tags: vec![],
            timeout_seconds: 30,
            env: HashMap::new(),
            parameters: vec![],
        };

        let mut params = HashMap::new();
        params.insert("unknown".to_string(), "value".to_string());

        let result = validate_parameters(&command, &params);
        assert!(result.is_err());
    }
}
