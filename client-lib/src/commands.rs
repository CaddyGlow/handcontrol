use crate::{
    certificates::CertificatePaths,
    grpc_client::connect_registered,
    proto::{
        session_client_message, session_server_message, Capability as ProtoCapability,
        CapabilityKind as ProtoCapabilityKind, CapabilityParameter as ProtoCapabilityParameter,
        CapabilityParameterType as ProtoCapabilityParameterType, ListCapabilitiesRequest,
        SessionClientMessage, SessionMode as ProtoSessionMode, SessionOpen,
    },
    storage::ServerRegistryEntry,
};
use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::convert::TryFrom;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Request;

/// Summary information about available capabilities on a server.
#[derive(Debug, Clone, Serialize)]
pub struct CommandList {
    pub config_version: u64,
    pub commands: Vec<CommandSummary>,
}

/// Metadata describing an executable capability (formerly command).
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
    pub privileged: bool,
    pub kind: CommandKind,
    pub session_mode: CommandSessionMode,
}

/// Parameter definition associated with a capability.
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
    pub default_value_command: Option<String>,
    pub default_value_pattern: Option<String>,
}

/// Supported parameter kinds.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandParameterType {
    Slider,
    Text,
    Toggle,
    Dropdown,
}

/// Runtime classification of a capability.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    ShellScript,
    ShellInteractive,
    FileTransfer,
    Unknown,
}

/// Session behaviour required by a capability.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandSessionMode {
    OneShot,
    Realtime,
    Upload,
    Download,
    Unknown,
}

/// Streaming events emitted while executing a capability.
#[derive(Debug, Clone)]
pub enum CommandStreamEvent {
    Stdout(String),
    Stderr(String),
}

impl CommandSummary {
    fn from_proto(proto: ProtoCapability) -> Result<Self> {
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
            icon: None,
            tags: proto.tags,
            parameters,
            requires_confirmation: proto.requires_confirmation.unwrap_or(false),
            show_output: true,
            privileged: proto.privileged.unwrap_or(false),
            kind: CommandKind::from_proto(proto.kind),
            session_mode: CommandSessionMode::from_proto(proto.session_mode),
        })
    }

    /// Returns true when the capability behaves like a traditional one-shot command.
    pub fn is_shell_script_oneshot(&self) -> bool {
        self.kind == CommandKind::ShellScript && self.session_mode == CommandSessionMode::OneShot
    }
}

impl CommandKind {
    fn from_proto(value: i32) -> Self {
        match ProtoCapabilityKind::try_from(value).unwrap_or(ProtoCapabilityKind::Unspecified) {
            ProtoCapabilityKind::ShellScript => CommandKind::ShellScript,
            ProtoCapabilityKind::ShellInteractive => CommandKind::ShellInteractive,
            ProtoCapabilityKind::FileTransfer => CommandKind::FileTransfer,
            ProtoCapabilityKind::Unspecified => CommandKind::Unknown,
        }
    }
}

impl CommandSessionMode {
    fn from_proto(value: i32) -> Self {
        match ProtoSessionMode::try_from(value).unwrap_or(ProtoSessionMode::Unspecified) {
            ProtoSessionMode::OneShot => CommandSessionMode::OneShot,
            ProtoSessionMode::Realtime => CommandSessionMode::Realtime,
            ProtoSessionMode::Upload => CommandSessionMode::Upload,
            ProtoSessionMode::Download => CommandSessionMode::Download,
            ProtoSessionMode::Unspecified => CommandSessionMode::Unknown,
        }
    }
}

impl CommandParameter {
    fn from_proto(proto: ProtoCapabilityParameter) -> Result<Self> {
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
            default_value_command: proto.default_value_command.filter(|v| !v.is_empty()),
            default_value_pattern: proto.default_value_pattern.filter(|v| !v.is_empty()),
        })
    }
}

impl CommandParameterType {
    fn from_proto(value: i32) -> Result<Self> {
        match ProtoCapabilityParameterType::try_from(value)
            .unwrap_or(ProtoCapabilityParameterType::Unspecified)
        {
            ProtoCapabilityParameterType::Slider => Ok(CommandParameterType::Slider),
            ProtoCapabilityParameterType::Text => Ok(CommandParameterType::Text),
            ProtoCapabilityParameterType::Toggle => Ok(CommandParameterType::Toggle),
            ProtoCapabilityParameterType::Dropdown => Ok(CommandParameterType::Dropdown),
            ProtoCapabilityParameterType::Unspecified => {
                bail!("Encountered parameter with unspecified type")
            }
        }
    }
}

/// Retrieve capabilities from a server using stored credentials.
pub async fn list_commands(entry: &ServerRegistryEntry) -> Result<CommandList> {
    let cert_paths = CertificatePaths::for_server(&entry.id)?;
    let mut client = connect_registered(entry, &cert_paths).await?;

    let response = client
        .inner()
        .list_capabilities(Request::new(ListCapabilitiesRequest {}))
        .await
        .context("ListCapabilities RPC failed")?
        .into_inner();

    let commands = response
        .capabilities
        .into_iter()
        .map(CommandSummary::from_proto)
        .collect::<Result<Vec<_>>>()?;

    Ok(CommandList {
        config_version: response.config_version,
        commands,
    })
}

/// Validate parameters for a capability, returning a sanitized map for RPC transmission.
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
                        if !param
                            .options
                            .iter()
                            .any(|opt| opt.eq_ignore_ascii_case(value))
                        {
                            bail!(
                                "Parameter '{}' must be one of: {}",
                                param.name,
                                param.options.join(", ")
                            );
                        }
                        param
                            .options
                            .iter()
                            .find(|opt| opt.eq_ignore_ascii_case(value))
                            .unwrap()
                            .to_string()
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

/// Execute a one-shot capability, invoking the callback for each stdout/stderr chunk.
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

    let open_message = SessionClientMessage {
        session_id: String::new(),
        payload: Some(session_client_message::Payload::Open(SessionOpen {
            capability_id: command_id.to_string(),
            parameters: request_parameters,
            request_id: None,
        })),
    };

    let (tx, rx) = mpsc::channel(8);
    tx.send(open_message)
        .await
        .context("Failed to send session open message")?;
    drop(tx);

    let request_stream = ReceiverStream::new(rx);
    let mut stream = client
        .inner()
        .open_session(Request::new(request_stream))
        .await
        .context("OpenSession RPC failed")?
        .into_inner();

    let mut exit_code: Option<i32> = None;

    while let Some(message) = stream
        .message()
        .await
        .context("Capability stream closed unexpectedly")?
    {
        match message.payload {
            Some(session_server_message::Payload::Ready(ready)) => {
                let session_mode = CommandSessionMode::from_proto(ready.session_mode);
                if session_mode != CommandSessionMode::OneShot {
                    bail!(
                        "Capability '{}' requires '{:?}' sessions which are not supported by this client",
                        command_id,
                        session_mode
                    );
                }
            }
            Some(session_server_message::Payload::Output(output)) => {
                let text = String::from_utf8_lossy(&output.data).to_string();
                if output.stderr.unwrap_or(false) {
                    on_event(CommandStreamEvent::Stderr(text));
                } else {
                    on_event(CommandStreamEvent::Stdout(text));
                }
            }
            Some(session_server_message::Payload::Exit(exit)) => {
                exit_code = Some(exit.exit_code);
                if let Some(message) = exit.message.filter(|m| !m.is_empty()) {
                    on_event(CommandStreamEvent::Stderr(format!("{message}\n")));
                }
            }
            Some(session_server_message::Payload::Error(err)) => {
                bail!("Server reported capability error: {}", err.message);
            }
            Some(session_server_message::Payload::Heartbeat(_)) => {
                // Ignore heartbeat acknowledgements for one-shot sessions.
            }
            Some(session_server_message::Payload::Closed(_)) => {
                break;
            }
            None => continue,
        }
    }

    exit_code.ok_or_else(|| anyhow!("Capability stream ended without exit code"))
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}
