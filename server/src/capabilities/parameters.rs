use crate::config::parser::{CapabilityConfig, CapabilityDefinition, CapabilityParameterConfig};
use anyhow::{Context, Result, anyhow};
use regex::Regex;
use std::collections::HashMap;

/// Validate parameter values against capability definition
pub fn validate_parameters(
    capability: &CapabilityConfig,
    provided: &HashMap<String, String>,
) -> Result<HashMap<String, String>> {
    let mut validated = HashMap::new();

    for param in &capability.parameters {
        let value = if let Some(v) = provided.get(&param.name) {
            v.clone()
        } else if let Some(default) = &param.default {
            default.clone()
        } else {
            return Err(anyhow!(
                "Missing required parameter '{}' for capability '{}'",
                param.name,
                capability.id
            ));
        };

        validate_parameter_value(capability, param, &value)?;
        validated.insert(param.name.clone(), value);
    }

    for key in provided.keys() {
        if !capability.parameters.iter().any(|p| &p.name == key) {
            return Err(anyhow!(
                "Unknown parameter '{}' for capability '{}'",
                key,
                capability.id
            ));
        }
    }

    Ok(validated)
}

fn validate_parameter_value(
    capability: &CapabilityConfig,
    param: &CapabilityParameterConfig,
    value: &str,
) -> Result<()> {
    match param.param_type.as_str() {
        "slider" => {
            let num: i32 = value
                .parse()
                .context("Slider value must be a valid integer")?;
            if let Some(min) = param.min {
                if num < min {
                    return Err(anyhow!(
                        "Value {} for parameter '{}' is below minimum {} on capability '{}'",
                        num,
                        param.name,
                        min,
                        capability.id
                    ));
                }
            }
            if let Some(max) = param.max {
                if num > max {
                    return Err(anyhow!(
                        "Value {} for parameter '{}' is above maximum {} on capability '{}'",
                        num,
                        param.name,
                        max,
                        capability.id
                    ));
                }
            }
        }
        "toggle" => match value {
            "true" | "false" | "1" | "0" => {}
            _ => {
                return Err(anyhow!(
                    "Toggle parameter '{}' on capability '{}' must be one of true/false/1/0",
                    param.name,
                    capability.id
                ));
            }
        },
        "dropdown" => {
            if !param.options.iter().any(|opt| opt == value) {
                return Err(anyhow!(
                    "Parameter '{}' on capability '{}' received invalid option '{}'",
                    param.name,
                    capability.id,
                    value
                ));
            }
        }
        "text" => {
            if let Some(pattern) = &param.validation {
                let regex = Regex::new(pattern)
                    .context("Invalid regex defined for parameter validation")?;
                if !regex.is_match(value) {
                    return Err(anyhow!(
                        "Parameter '{}' on capability '{}' does not match pattern {}",
                        param.name,
                        capability.id,
                        pattern
                    ));
                }
            }
        }
        other => {
            return Err(anyhow!(
                "Capability '{}' parameter '{}' has unsupported type '{}'",
                capability.id,
                param.name,
                other
            ));
        }
    }
    Ok(())
}

pub fn substitute_parameters(
    capability: &CapabilityConfig,
    parameters: &HashMap<String, String>,
) -> Result<String> {
    let command = match &capability.definition {
        CapabilityDefinition::ShellScript(def) => &def.command,
        other => {
            return Err(anyhow!(
                "Capability '{}' of type {:?} does not support parameter substitution",
                capability.id,
                other
            ));
        }
    };

    let mut result = command.clone();
    let re = Regex::new(r"\{([a-zA-Z0-9_-]+)\}").context("Failed to compile placeholder regex")?;

    for cap in re.captures_iter(command) {
        let placeholder = cap.get(0).unwrap().as_str();
        let name = cap.get(1).unwrap().as_str();
        let value = parameters
            .get(name)
            .ok_or_else(|| anyhow!("Parameter '{}' not provided", name))?;
        let escaped = shell_escape::escape(value.into());
        result = result.replace(placeholder, &escaped);
    }

    Ok(result)
}
