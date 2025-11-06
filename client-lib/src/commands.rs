use crate::{
    certificates::CertificatePaths,
    grpc_client::connect_registered,
    proto::{
        session_client_message, session_server_message, Capability as ProtoCapability,
        CapabilityKind as ProtoCapabilityKind, CapabilityParameter as ProtoCapabilityParameter,
        CapabilityParameterType as ProtoCapabilityParameterType, ListCapabilitiesRequest,
        SessionClientMessage, SessionClose, SessionHeartbeat, SessionInput,
        SessionMode as ProtoSessionMode, SessionOpen, SessionResize, SessionResume,
        SessionServerMessage,
    },
    storage::ServerRegistryEntry,
};
use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::TryFrom;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Streaming};

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

impl CapabilitySession {
    /// Returns the session identifier assigned by the server.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Returns the capability identifier associated with this session.
    pub fn capability_id(&self) -> &str {
        &self.capability_id
    }

    /// Returns the session mode reported by the server.
    pub fn session_mode(&self) -> CommandSessionMode {
        self.session_mode
    }

    /// Optional human-readable message from the capability when the session was readied.
    pub fn ready_message(&self) -> Option<&str> {
        self.ready_message.as_deref()
    }

    /// Returns the current resume token issued by the server.
    pub fn resume_token(&self) -> &str {
        &self.resume_token
    }

    /// Returns the last stdout sequence observed for replay alignment.
    pub fn last_stdout_sequence(&self) -> u64 {
        self.last_stdout_sequence
    }

    /// Returns the last stderr sequence observed for replay alignment.
    pub fn last_stderr_sequence(&self) -> u64 {
        self.last_stderr_sequence
    }

    /// Produces a snapshot of the session suitable for resumable reconnects.
    pub fn resume_state(&self) -> CapabilitySessionResumeState {
        CapabilitySessionResumeState {
            session_id: self.session_id.clone(),
            capability_id: self.capability_id.clone(),
            resume_token: self.resume_token.clone(),
            last_stdout_sequence: self.last_stdout_sequence,
            last_stderr_sequence: self.last_stderr_sequence,
            session_mode: self.session_mode,
        }
    }

    /// Returns a cloneable sender for pushing input/control messages to the server.
    pub fn sender(&self) -> CapabilitySessionSender {
        CapabilitySessionSender {
            session_id: self.session_id.clone(),
            sender: self.sender.clone(),
        }
    }

    /// Receive the next event from the capability session.
    pub async fn recv(&mut self) -> Result<Option<CapabilitySessionEvent>> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(Some(event));
        }

        loop {
            let message = match self
                .stream
                .message()
                .await
                .context("Capability stream closed unexpectedly")?
            {
                None => return Ok(None),
                Some(message) => message,
            };

            match message.payload {
                Some(session_server_message::Payload::Output(output)) => {
                    let stderr = output.stderr.unwrap_or(false);
                    let sequence = output.sequence;
                    if sequence != 0 {
                        if stderr {
                            self.last_stderr_sequence = sequence;
                        } else {
                            self.last_stdout_sequence = sequence;
                        }
                    }
                    return Ok(Some(CapabilitySessionEvent::Output {
                        data: output.data,
                        stderr,
                        binary: output.binary.unwrap_or(false),
                        timestamp_ms: output.timestamp_ms,
                        sequence,
                    }));
                }
                Some(session_server_message::Payload::Exit(exit)) => {
                    return Ok(Some(CapabilitySessionEvent::Exit {
                        exit_code: exit.exit_code,
                        timed_out: exit.timed_out.unwrap_or(false),
                        message: exit.message,
                    }));
                }
                Some(session_server_message::Payload::Error(err)) => {
                    return Ok(Some(CapabilitySessionEvent::Error {
                        message: err.message,
                        code: err.code,
                    }));
                }
                Some(session_server_message::Payload::Heartbeat(ack)) => {
                    return Ok(Some(CapabilitySessionEvent::HeartbeatAck {
                        timestamp_ms: ack.timestamp_ms,
                        latency_hint_ms: ack.latency_hint_ms,
                    }));
                }
                Some(session_server_message::Payload::Closed(closed)) => {
                    return Ok(Some(CapabilitySessionEvent::Closed {
                        reason: closed.reason,
                    }));
                }
                Some(session_server_message::Payload::ResumeAck(ack)) => {
                    if !ack.resume_token.is_empty() {
                        self.resume_token = ack.resume_token;
                    }
                    continue;
                }
                Some(session_server_message::Payload::Ready(_)) => {
                    // Should not arrive after initial handshake; ignore gracefully.
                    continue;
                }
                None => continue,
            }
        }
    }
}

impl CapabilitySessionSender {
    async fn send_message(&self, payload: session_client_message::Payload) -> Result<()> {
        self.sender
            .send(SessionClientMessage {
                session_id: self.session_id.clone(),
                payload: Some(payload),
            })
            .await
            .context("Failed to send capability session message")
    }

    /// Send input bytes to the capability session.
    pub async fn send_input(&self, data: Vec<u8>, binary: bool) -> Result<()> {
        self.send_message(session_client_message::Payload::Input(SessionInput {
            data,
            binary: Some(binary),
        }))
        .await
    }

    /// Notify the capability session about terminal size changes.
    pub async fn send_resize(&self, cols: u32, rows: u32) -> Result<()> {
        self.send_message(session_client_message::Payload::Resize(SessionResize {
            cols,
            rows,
        }))
        .await
    }

    /// Send a heartbeat to keep the session alive.
    pub async fn send_heartbeat(&self, timestamp_ms: i64) -> Result<()> {
        self.send_message(session_client_message::Payload::Heartbeat(
            SessionHeartbeat { timestamp_ms },
        ))
        .await
    }

    /// Request graceful session shutdown.
    pub async fn close(&self, reason: Option<String>) -> Result<()> {
        self.send_message(session_client_message::Payload::Close(SessionClose {
            reason,
        }))
        .await
    }
}

/// Bidirectional capability session handle returned by `open_capability_session`.
#[derive(Debug)]
pub struct CapabilitySession {
    session_id: String,
    capability_id: String,
    session_mode: CommandSessionMode,
    ready_message: Option<String>,
    sender: mpsc::Sender<SessionClientMessage>,
    stream: Streaming<SessionServerMessage>,
    resume_token: String,
    last_stdout_sequence: u64,
    last_stderr_sequence: u64,
    pending_events: VecDeque<CapabilitySessionEvent>,
}

/// Cloneable helper for sending input/control messages to an active capability session.
#[derive(Clone, Debug)]
pub struct CapabilitySessionSender {
    session_id: String,
    sender: mpsc::Sender<SessionClientMessage>,
}

/// Snapshot of resumable session state for reconnect attempts.
#[derive(Debug, Clone)]
pub struct CapabilitySessionResumeState {
    pub session_id: String,
    pub capability_id: String,
    pub resume_token: String,
    pub last_stdout_sequence: u64,
    pub last_stderr_sequence: u64,
    pub session_mode: CommandSessionMode,
}

/// Events emitted by an active capability session.
#[derive(Debug)]
pub enum CapabilitySessionEvent {
    Output {
        data: Vec<u8>,
        stderr: bool,
        binary: bool,
        timestamp_ms: Option<i64>,
        sequence: u64,
    },
    Exit {
        exit_code: i32,
        timed_out: bool,
        message: Option<String>,
    },
    Error {
        message: String,
        code: Option<i32>,
    },
    HeartbeatAck {
        timestamp_ms: i64,
        latency_hint_ms: Option<i64>,
    },
    Closed {
        reason: Option<String>,
    },
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

/// Open a capability session and return a bidirectional handle.
pub async fn open_capability_session(
    entry: &ServerRegistryEntry,
    capability_id: &str,
    parameters: HashMap<String, String>,
) -> Result<CapabilitySession> {
    let cert_paths = CertificatePaths::for_server(&entry.id)?;
    let mut client = connect_registered(entry, &cert_paths).await?;

    let (tx, rx) = mpsc::channel(32);
    tx.send(SessionClientMessage {
        session_id: String::new(),
        payload: Some(session_client_message::Payload::Open(SessionOpen {
            capability_id: capability_id.to_string(),
            parameters,
            request_id: None,
        })),
    })
    .await
    .context("Failed to send session open message")?;

    let request_stream = ReceiverStream::new(rx);
    let mut stream = client
        .inner()
        .open_session(Request::new(request_stream))
        .await
        .context("OpenSession RPC failed")?
        .into_inner();

    let (session_id, ready) = loop {
        let message = stream
            .message()
            .await
            .context("Capability stream closed unexpectedly")?
            .ok_or_else(|| anyhow!("Capability stream ended before ready message"))?;

        match message.payload {
            Some(session_server_message::Payload::Ready(ready)) => {
                let session_id = if message.session_id.is_empty() {
                    bail!("Session ready message missing session id");
                } else {
                    message.session_id
                };
                break (session_id, ready);
            }
            Some(session_server_message::Payload::Error(err)) => {
                bail!("Server reported capability error: {}", err.message);
            }
            Some(session_server_message::Payload::Closed(closed)) => {
                let reason = closed
                    .reason
                    .unwrap_or_else(|| "session closed before ready".to_string());
                bail!(reason);
            }
            Some(session_server_message::Payload::Heartbeat(_)) => {
                continue;
            }
            Some(other) => {
                bail!(
                    "Received unexpected session payload before ready: {:?}",
                    other
                );
            }
            None => continue,
        }
    };

    let session_mode = CommandSessionMode::from_proto(ready.session_mode);
    if session_mode == CommandSessionMode::Unknown {
        bail!(
            "Capability '{}' reported unknown session mode",
            capability_id
        );
    }

    Ok(CapabilitySession {
        session_id,
        capability_id: if ready.capability_id.is_empty() {
            capability_id.to_string()
        } else {
            ready.capability_id
        },
        session_mode,
        ready_message: ready.message.filter(|m| !m.is_empty()),
        sender: tx,
        stream,
        resume_token: ready.resume_token,
        last_stdout_sequence: 0,
        last_stderr_sequence: 0,
        pending_events: VecDeque::new(),
    })
}

/// Resume an existing capability session using previously captured resume state.
pub async fn resume_capability_session(
    entry: &ServerRegistryEntry,
    state: CapabilitySessionResumeState,
) -> Result<CapabilitySession> {
    let CapabilitySessionResumeState {
        session_id,
        capability_id,
        resume_token: saved_resume_token,
        mut last_stdout_sequence,
        mut last_stderr_sequence,
        session_mode,
    } = state;

    let cert_paths = CertificatePaths::for_server(&entry.id)?;
    let mut client = connect_registered(entry, &cert_paths).await?;

    let (tx, rx) = mpsc::channel(32);
    tx.send(SessionClientMessage {
        session_id: session_id.clone(),
        payload: Some(session_client_message::Payload::Resume(SessionResume {
            resume_token: saved_resume_token.clone(),
            last_output_sequence: (last_stdout_sequence > 0).then_some(last_stdout_sequence),
            last_error_sequence: (last_stderr_sequence > 0).then_some(last_stderr_sequence),
        })),
    })
    .await
    .context("Failed to send session resume message")?;

    let request_stream = ReceiverStream::new(rx);
    let mut stream = client
        .inner()
        .open_session(Request::new(request_stream))
        .await
        .context("OpenSession RPC failed")?
        .into_inner();

    let mut pending_events = VecDeque::new();
    let mut resume_token = saved_resume_token;

    loop {
        let message = stream
            .message()
            .await
            .context("Capability stream closed unexpectedly during resume")?
            .ok_or_else(|| anyhow!("Capability stream ended before resume acknowledgement"))?;

        if !message.session_id.is_empty() && message.session_id != session_id {
            bail!(
                "Received session message for mismatched session_id={} (expected {})",
                message.session_id,
                session_id
            );
        }

        match message.payload {
            Some(session_server_message::Payload::ResumeAck(ack)) => {
                if !ack.resume_token.is_empty() {
                    resume_token = ack.resume_token;
                }
                break;
            }
            Some(session_server_message::Payload::Output(output)) => {
                let stderr = output.stderr.unwrap_or(false);
                let sequence = output.sequence;
                if sequence != 0 {
                    if stderr {
                        last_stderr_sequence = sequence;
                    } else {
                        last_stdout_sequence = sequence;
                    }
                }
                pending_events.push_back(CapabilitySessionEvent::Output {
                    data: output.data,
                    stderr,
                    binary: output.binary.unwrap_or(false),
                    timestamp_ms: output.timestamp_ms,
                    sequence,
                });
            }
            Some(session_server_message::Payload::Error(err)) => {
                bail!(
                    "Server reported capability error while resuming session: {}",
                    err.message
                );
            }
            Some(session_server_message::Payload::Closed(closed)) => {
                let reason = closed
                    .reason
                    .unwrap_or_else(|| "session closed while resuming".to_string());
                bail!(reason);
            }
            Some(session_server_message::Payload::Exit(exit)) => {
                let message = exit
                    .message
                    .unwrap_or_else(|| "session exited while resuming".to_string());
                bail!(
                    "Capability session exited (code {}): {}",
                    exit.exit_code,
                    message
                );
            }
            Some(session_server_message::Payload::Heartbeat(_)) => continue,
            Some(session_server_message::Payload::Ready(_)) => continue,
            None => continue,
        }
    }

    Ok(CapabilitySession {
        session_id,
        capability_id,
        session_mode,
        ready_message: None,
        sender: tx,
        stream,
        resume_token,
        last_stdout_sequence,
        last_stderr_sequence,
        pending_events,
    })
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
    let mut session = open_capability_session(entry, command_id, parameters).await?;

    if session.session_mode() != CommandSessionMode::OneShot {
        bail!(
            "Capability '{}' requires '{:?}' sessions which are not supported by this client",
            command_id,
            session.session_mode()
        );
    }

    let mut exit_code: Option<i32> = None;

    while let Some(event) = session.recv().await? {
        match event {
            CapabilitySessionEvent::Output { data, stderr, .. } => {
                let text = String::from_utf8_lossy(&data).to_string();
                if stderr {
                    on_event(CommandStreamEvent::Stderr(text));
                } else {
                    on_event(CommandStreamEvent::Stdout(text));
                }
            }
            CapabilitySessionEvent::Exit {
                exit_code: code,
                timed_out,
                message,
            } => {
                exit_code = Some(code);
                if timed_out {
                    on_event(CommandStreamEvent::Stderr(
                        "Session timed out\n".to_string(),
                    ));
                }
                if let Some(message) = message.filter(|m| !m.is_empty()) {
                    on_event(CommandStreamEvent::Stderr(format!("{message}\n")));
                }
            }
            CapabilitySessionEvent::Error { message, .. } => {
                bail!("Server reported capability error: {}", message);
            }
            CapabilitySessionEvent::HeartbeatAck { .. } => {
                // Ignore keep-alive acknowledgements for one-shot sessions.
            }
            CapabilitySessionEvent::Closed { reason } => {
                if exit_code.is_some() {
                    break;
                }
                if let Some(reason) = reason {
                    bail!("Capability session closed early: {}", reason);
                } else {
                    bail!("Capability session closed unexpectedly");
                }
            }
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
