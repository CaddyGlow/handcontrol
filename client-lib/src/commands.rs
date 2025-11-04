use crate::{
    certificates::CertificatePaths,
    grpc_client::connect_registered,
    proto::{
        execute_command_response, Command as ProtoCommand, ExecuteCommandRequest,
        ListCommandsRequest, Parameter as ProtoParameter, ParameterType,
    },
    storage::ServerRegistryEntry,
};
use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use tonic::Request;

/// Summary information about available commands on a server.
#[derive(Debug, Clone, Serialize)]
pub struct CommandList {
    pub config_version: u64,
    pub commands: Vec<CommandSummary>,
}

/// Command metadata returned by `list_commands`.
#[derive(Debug, Clone, Serialize)]
pub struct CommandSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub tags: Vec<String>,
    pub parameters: Vec<CommandParameter>,
    pub requires_confirmation: bool,
    pub show_output: bool,
}

/// Parameter definition for a command.
#[derive(Debug, Clone, Serialize)]
pub struct CommandParameter {
    pub name: String,
    pub description: Option<String>,
    pub param_type: CommandParameterType,
    pub min: Option<i32>,
    pub max: Option<i32>,
    pub default_value: Option<String>,
    pub validation: Option<String>,
    pub options: Vec<String>,
    pub label_on: Option<String>,
    pub label_off: Option<String>,
}

/// Supported parameter types.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandParameterType {
    Slider,
    Text,
    Toggle,
    Dropdown,
}

/// Streaming events emitted while executing a command.
#[derive(Debug, Clone)]
pub enum CommandStreamEvent {
    Stdout(String),
    Stderr(String),
}

/// Retrieve the list of commands from a server using stored credentials.
pub async fn list_commands(entry: &ServerRegistryEntry) -> Result<CommandList> {
    let cert_paths = CertificatePaths::for_server(&entry.id)?;
    let mut client = connect_registered(entry, &cert_paths).await?;

    let response = client
        .inner()
        .list_commands(Request::new(ListCommandsRequest {}))
        .await
        .context("ListCommands RPC failed")?
        .into_inner();

    let commands = response
        .commands
        .into_iter()
        .map(CommandSummary::from_proto)
        .collect::<Result<Vec<_>>>()?;

    Ok(CommandList {
        config_version: response.config_version,
        commands,
    })
}

/// Validate parameters for a command, returning a sanitized map suitable for RPC transmission.
pub fn validate_parameters(
    command: &CommandSummary,
    provided: &HashMap<String, String>,
) -> Result<HashMap<String, String>> {
    let known_params: HashSet<&str> = command.parameters.iter().map(|p| p.name.as_str()).collect();
    for key in provided.keys() {
        if !known_params.contains(key.as_str()) {
            bail!("Unknown parameter '{}'", key);
        }
    }

    let mut sanitized = HashMap::new();
    for param in &command.parameters {
        match provided.get(&param.name) {
            Some(raw) => {
                let value = raw.trim();
                let normalized = match param.param_type {
                    CommandParameterType::Slider => {
                        let parsed: i32 = value.parse().with_context(|| {
                            format!("Parameter '{}' must be an integer", param.name)
                        })?;
                        if let Some(min) = param.min {
                            if parsed < min {
                                bail!(
                                    "Parameter '{}' must be >= {} (got {})",
                                    param.name,
                                    min,
                                    parsed
                                );
                            }
                        }
                        if let Some(max) = param.max {
                            if parsed > max {
                                bail!(
                                    "Parameter '{}' must be <= {} (got {})",
                                    param.name,
                                    max,
                                    parsed
                                );
                            }
                        }
                        parsed.to_string()
                    }
                    CommandParameterType::Toggle => match parse_bool(value) {
                        Some(true) => "true".to_string(),
                        Some(false) => "false".to_string(),
                        None => bail!(
                            "Parameter '{}' must be true/false, yes/no, on/off, or 1/0 (got '{}')",
                            param.name,
                            value
                        ),
                    },
                    CommandParameterType::Dropdown => {
                        if param.options.is_empty() {
                            bail!(
                                "Dropdown parameter '{}' has no configured options",
                                param.name
                            );
                        }
                        if !param.options.iter().any(|opt| opt == value) {
                            bail!(
                                "Parameter '{}' must be one of: {}",
                                param.name,
                                param.options.join(", ")
                            );
                        }
                        value.to_string()
                    }
                    CommandParameterType::Text => {
                        if let Some(pattern) = &param.validation {
                            let regex = Regex::new(pattern).with_context(|| {
                                format!("Invalid regex for parameter '{}': {}", param.name, pattern)
                            })?;
                            if !regex.is_match(value) {
                                bail!(
                                    "Parameter '{}' does not match required pattern '{}'",
                                    param.name,
                                    pattern
                                );
                            }
                        }
                        value.to_string()
                    }
                };
                sanitized.insert(param.name.clone(), normalized);
            }
            None => {
                if param.default_value.is_none() {
                    bail!("Missing required parameter '{}'", param.name);
                }
            }
        }
    }

    Ok(sanitized)
}

/// Execute a command, invoking the callback for each stdout/stderr chunk.
pub async fn execute_command<F>(
    entry: &ServerRegistryEntry,
    command_id: &str,
    parameters: HashMap<String, String>,
    mut on_event: F,
) -> Result<i32>
where
    F: FnMut(CommandStreamEvent),
{
    let cert_paths = CertificatePaths::for_server(&entry.id)?;
    let mut client = connect_registered(entry, &cert_paths).await?;

    let mut request_parameters = HashMap::new();
    for (key, value) in parameters {
        request_parameters.insert(key, value);
    }

    let request = ExecuteCommandRequest {
        command_id: command_id.to_string(),
        parameters: request_parameters,
    };

    let mut stream = client
        .inner()
        .execute_command(Request::new(request))
        .await
        .context("ExecuteCommand RPC failed")?
        .into_inner();

    let mut exit_code: Option<i32> = None;

    while let Some(message) = stream
        .message()
        .await
        .context("Command stream closed unexpectedly")?
    {
        match message.response {
            Some(execute_command_response::Response::Stdout(data)) => {
                on_event(CommandStreamEvent::Stdout(data));
            }
            Some(execute_command_response::Response::Stderr(data)) => {
                on_event(CommandStreamEvent::Stderr(data));
            }
            Some(execute_command_response::Response::ExitCode(code)) => {
                exit_code = Some(code);
                break;
            }
            Some(execute_command_response::Response::Error(err)) => {
                bail!("Server reported command error: {}", err);
            }
            None => continue,
        }
    }

    exit_code.ok_or_else(|| anyhow!("Command stream ended without exit code"))
}

impl CommandSummary {
    fn from_proto(proto: ProtoCommand) -> Result<Self> {
        let parameters = proto
            .parameters
            .into_iter()
            .map(CommandParameter::from_proto)
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            id: proto.id,
            name: proto.name,
            description: if proto.description.is_empty() {
                None
            } else {
                Some(proto.description)
            },
            icon: if proto.icon.is_empty() {
                None
            } else {
                Some(proto.icon)
            },
            tags: proto.tags,
            parameters,
            requires_confirmation: proto.requires_confirmation.unwrap_or(false),
            show_output: proto.show_output.unwrap_or(true),
        })
    }
}

impl CommandParameter {
    fn from_proto(proto: ProtoParameter) -> Result<Self> {
        let param_type = CommandParameterType::from_proto(proto.r#type)?;
        Ok(Self {
            name: proto.name,
            description: if proto.description.is_empty() {
                None
            } else {
                Some(proto.description)
            },
            param_type,
            min: proto.min,
            max: proto.max,
            default_value: proto.default_value.filter(|v| !v.is_empty()),
            validation: proto.validation.filter(|v| !v.is_empty()),
            options: proto.options,
            label_on: proto.label_on.filter(|v| !v.is_empty()),
            label_off: proto.label_off.filter(|v| !v.is_empty()),
        })
    }
}

impl CommandParameterType {
    fn from_proto(value: i32) -> Result<Self> {
        match ParameterType::try_from(value).unwrap_or(ParameterType::Unspecified) {
            ParameterType::Slider => Ok(CommandParameterType::Slider),
            ParameterType::Text => Ok(CommandParameterType::Text),
            ParameterType::Toggle => Ok(CommandParameterType::Toggle),
            ParameterType::Dropdown => Ok(CommandParameterType::Dropdown),
            ParameterType::Unspecified => bail!("Encountered parameter with unspecified type"),
        }
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}
