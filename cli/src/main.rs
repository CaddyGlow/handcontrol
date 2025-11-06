use anyhow::{anyhow, bail, Context, Result};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};
use handcontrol_client_lib::{
    config::{self, ClientConfig, DeviceConfig, TransportPreference, TRANSPORT_OVERRIDE_ENV},
    discover_servers, enroll_via_approval, enroll_via_qr, execute_command, fetch_server_info,
    list_commands as fetch_command_list, list_sessions as fetch_sessions, open_capability_session,
    resume_capability_session,
    storage::{ServerRegistry, ServerRegistryEntry},
    validate_parameters, ApprovalEnrollmentInput, CapabilitySession, CapabilitySessionEvent,
    CapabilitySessionResumeState, CapabilitySessionSender, CommandKind, CommandList,
    CommandSessionMode, CommandStreamEvent, CommandSummary, DiscoveredServer, QrEnrollmentInput,
    ServerSessionInfo,
};
use rustls::crypto;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::IsTerminal;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use time::{format_description::well_known::Rfc3339, Duration as TimeDuration, OffsetDateTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use uuid::Uuid;

use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size as terminal_size};

#[cfg(unix)]
use tokio::signal::unix::{signal, SignalKind};

#[derive(Parser)]
#[command(
    name = "handcontrol-cli",
    author,
    version,
    about = "HandControl command-line client",
    long_about = "HandControl CLI provides terminal-based access to HandControl servers."
)]
struct Cli {
    /// Override the relay transport to use for this invocation
    #[arg(long, value_enum)]
    transport: Option<TransportCliChoice>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum TransportCliChoice {
    Auto,
    Websocket,
    Quic,
}

impl TransportCliChoice {
    fn as_env_value(&self) -> &'static str {
        match self {
            TransportCliChoice::Auto => "auto",
            TransportCliChoice::Websocket => "websocket",
            TransportCliChoice::Quic => "quic",
        }
    }
}

impl From<TransportCliChoice> for TransportPreference {
    fn from(value: TransportCliChoice) -> Self {
        match value {
            TransportCliChoice::Auto => TransportPreference::Auto,
            TransportCliChoice::Websocket => TransportPreference::Websocket,
            TransportCliChoice::Quic => TransportPreference::Quic,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Discover servers announced via mDNS
    Discover(DiscoverCommand),
    /// List enrolled servers
    ListServers(ListServersCommand),
    /// Show detailed information about a server
    Info(InfoCommand),
    /// List commands available on an enrolled server
    List(ListCommand),
    /// Execute a command on an enrolled server
    Exec(ExecCommand),
    /// Inspect active sessions on an enrolled server
    Sessions(SessionsCommand),
    /// Resume an existing interactive session
    Resume(ResumeCommand),
    /// Generate shell completion scripts
    Completions(CompletionCommand),
    /// Remove stored enrollment for a server
    Remove(RemoveCommand),
    /// Manage client configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Enroll the CLI client with a server
    Enroll {
        #[command(subcommand)]
        command: EnrollCommand,
    },
}

#[derive(Parser)]
struct DiscoverCommand {
    /// Override discovery timeout (seconds)
    #[arg(long)]
    timeout: Option<u64>,
    /// Output JSON instead of TSV
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct ListServersCommand {
    /// Output JSON instead of TSV
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct InfoCommand {
    /// Server identifier (UUID, instance name, or hostname)
    server: String,
    /// Direct server address if discovery is unavailable (host or host:port)
    #[arg(long)]
    address: Option<String>,
    /// Override port (defaults to 50051 or discovery result)
    #[arg(long)]
    port: Option<u16>,
    /// Output JSON instead of human-readable text
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct ListCommand {
    /// Server identifier (UUID, hostname, or alias)
    server: String,
    /// Filter commands by tag (repeatable)
    #[arg(long = "tag", value_name = "TAG")]
    tags: Vec<String>,
    /// Output JSON instead of TSV
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct ExecCommand {
    /// Server identifier (UUID, hostname, or alias)
    server: String,
    /// Command ID to execute
    command_id: String,
    /// Parameter key=value pairs
    #[arg(value_name = "key=value")]
    parameters: Vec<String>,
    /// Stream output as it arrives
    #[arg(long)]
    stream: bool,
    /// Suppress command output
    #[arg(long)]
    quiet: bool,
}

#[derive(Parser)]
struct RemoveCommand {
    /// Server identifier (UUID, hostname, or alias)
    server: String,
    /// Skip confirmation prompt
    #[arg(long, alias = "yes")]
    confirm: bool,
}

#[derive(Parser)]
struct SessionsCommand {
    /// Server identifier (UUID, hostname, or alias)
    server: String,
    /// Output JSON instead of a table
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct ResumeCommand {
    /// Server identifier (UUID, hostname, or alias)
    server: String,
    /// Session identifier (UUID)
    session_id: String,
}

#[derive(Parser)]
struct CompletionCommand {
    #[command(subcommand)]
    command: CompletionSubcommand,
}

#[derive(Subcommand)]
enum CompletionSubcommand {
    /// Generate shell completion scripts
    Generate(GenerateCompletionCommand),
    /// Internal helpers for dynamic shell completions
    #[command(hide = true)]
    Dynamic(DynamicCompletionCommand),
}

#[derive(Parser)]
struct GenerateCompletionCommand {
    /// Target shell to generate completions for
    #[arg(value_enum)]
    shell: Shell,
    /// Write the generated script to a file instead of stdout
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,
}

#[derive(Parser)]
struct DynamicCompletionCommand {
    #[command(subcommand)]
    target: DynamicCompletionTarget,
}

#[derive(Subcommand)]
enum DynamicCompletionTarget {
    /// Suggest enrolled server identifiers
    Servers {
        /// Optional prefix to filter results
        #[arg(value_name = "PREFIX")]
        prefix: Option<String>,
    },
    /// Suggest resumable session identifiers for a server
    Sessions {
        /// Server identifier (UUID, hostname, or alias)
        server: String,
        /// Optional prefix to filter results
        #[arg(value_name = "PREFIX")]
        prefix: Option<String>,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Show the current configuration
    Show {
        /// Output JSON instead of TOML
        #[arg(long)]
        json: bool,
    },
    /// Print the configuration file path
    Path,
    /// Update a configuration value using dotted keys (e.g. discovery.timeout_seconds)
    Set {
        /// Configuration key path (section.key)
        key: String,
        /// New value to assign
        value: String,
    },
}

#[derive(Subcommand)]
enum EnrollCommand {
    /// Enroll using a QR enrollment payload (copy/paste JSON)
    Qr(QrEnrollCommand),
    /// Enroll using the approval flow (verification code)
    Approve(ApproveEnrollCommand),
}

#[derive(Parser)]
struct QrEnrollCommand {
    /// Raw enrollment payload JSON
    #[arg(long, conflicts_with = "payload_file")]
    payload: Option<String>,
    /// Read enrollment payload JSON from a file
    #[arg(long)]
    payload_file: Option<PathBuf>,
    /// Override server ID from payload (UUID)
    #[arg(long)]
    server_id: Option<String>,
    /// Device name to present during enrollment
    #[arg(long)]
    device_name: Option<String>,
    /// Device model metadata (approval enrollment compatibility)
    #[arg(long)]
    device_model: Option<String>,
}

#[derive(Parser)]
struct ApproveEnrollCommand {
    /// Server identifier (UUID, instance name, or hostname)
    server: String,
    /// Direct server address if discovery is unavailable (host or host:port)
    #[arg(long)]
    address: Option<String>,
    /// Override port (defaults to 50051 or discovery result)
    #[arg(long)]
    port: Option<u16>,
    /// Enrollment timeout in seconds
    #[arg(long, default_value_t = 60)]
    timeout: u64,
    /// Poll interval in seconds while waiting for approval
    #[arg(long, default_value_t = 2)]
    poll_interval: u64,
    /// Device name to present during enrollment
    #[arg(long)]
    device_name: Option<String>,
    /// Device model metadata (optional)
    #[arg(long)]
    device_model: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = crypto::ring::default_provider().install_default();
    if tracing_subscriber::fmt::try_init().is_err() {
        debug!("Tracing subscriber already initialized");
    }

    let cli = Cli::parse();
    let transport_override = cli.transport;
    let command = cli.command;

    if let Some(choice) = transport_override {
        std::env::set_var(TRANSPORT_OVERRIDE_ENV, choice.as_env_value());
    }

    match command {
        Command::Discover(cmd) => run_discover(cmd),
        Command::ListServers(cmd) => run_list_servers(cmd),
        Command::Info(cmd) => run_info(cmd).await,
        Command::List(cmd) => run_list_commands(cmd).await,
        Command::Exec(cmd) => run_exec_command(cmd).await,
        Command::Sessions(cmd) => run_sessions(cmd).await,
        Command::Resume(cmd) => run_resume(cmd).await,
        Command::Completions(cmd) => run_completions(cmd).await,
        Command::Remove(cmd) => run_remove(cmd),
        Command::Config { command } => run_config(command),
        Command::Enroll { command } => run_enroll(command).await,
    }
}

fn run_discover(cmd: DiscoverCommand) -> Result<()> {
    let mut cfg = config::load()?;
    if let Some(timeout) = cmd.timeout {
        cfg.discovery.timeout_seconds = timeout;
    }

    let registry = ServerRegistry::load()?;
    let servers = discover_servers(&cfg.discovery)?;
    let mut rows: Vec<DiscoverRow> = servers
        .into_iter()
        .map(|server| to_discover_row(server, &registry))
        .collect();

    rows.sort_by(|a, b| a.instance_name.cmp(&b.instance_name));

    let as_json = if cmd.json {
        true
    } else {
        cfg.cli.output_format.eq_ignore_ascii_case("json")
    };

    if as_json {
        output_json(&rows)?;
    } else {
        output_tsv_discovery(&rows, &cfg)?;
    }

    Ok(())
}

fn run_list_servers(cmd: ListServersCommand) -> Result<()> {
    let cfg = config::load()?;
    let registry = ServerRegistry::load()?;
    let mut rows: Vec<RegistryRow> = registry.iter().map(RegistryRow::from).collect();
    rows.sort_by(|a, b| a.server_id.cmp(&b.server_id));

    let as_json = if cmd.json {
        true
    } else {
        cfg.cli.output_format.eq_ignore_ascii_case("json")
    };

    if as_json {
        output_json(&rows)?;
    } else {
        output_tsv_registry(&rows, &cfg)?;
    }

    Ok(())
}

async fn run_info(cmd: InfoCommand) -> Result<()> {
    let InfoCommand {
        server,
        address,
        port,
        json,
    } = cmd;

    let cfg = config::load()?;
    let resolved = resolve_server_targets(&server, address, port, &cfg)?;
    let ResolvedServer {
        addresses,
        port: resolved_port,
        server_id_hint,
    } = resolved;

    let port = resolved_port.unwrap_or(50051);
    let info = fetch_server_info(&addresses, port, server_id_hint).await?;

    let registry = ServerRegistry::load()?;
    let entry = registry.find_by_id(&info.server_id);
    let enrolled = entry.is_some();
    let client_id = entry.and_then(|e| e.client_id);

    let output = InfoOutput {
        server_id: info.server_id,
        hostname: if info.hostname.is_empty() {
            None
        } else {
            Some(info.hostname)
        },
        version: if info.version.is_empty() {
            None
        } else {
            Some(info.version)
        },
        os: if info.os.is_empty() {
            None
        } else {
            Some(info.os)
        },
        address: format_socket_address(&info.address, info.port),
        tls_fingerprint: info.fingerprint,
        enrolled,
        client_id,
    };

    let as_json = if json {
        true
    } else {
        cfg.cli.output_format.eq_ignore_ascii_case("json")
    };

    if as_json {
        output_json(&output)?;
    } else {
        print_info_output(&output);
    }

    Ok(())
}

async fn run_list_commands(cmd: ListCommand) -> Result<()> {
    let ListCommand { server, tags, json } = cmd;

    let cfg = config::load()?;
    let registry = ServerRegistry::load()?;
    let entry = find_enrolled_server(&server, &registry)?;

    let CommandList {
        config_version,
        mut commands,
    } = fetch_command_list(entry).await?;
    if !tags.is_empty() {
        let filters: Vec<String> = tags.iter().map(|t| t.to_ascii_lowercase()).collect();
        commands.retain(|cmd| {
            filters
                .iter()
                .all(|tag| cmd.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)))
        });
    }

    let output = CommandsOutput {
        server_id: entry.id,
        config_version,
        commands,
    };

    let as_json = if json {
        true
    } else {
        cfg.cli.output_format.eq_ignore_ascii_case("json")
    };

    if as_json {
        output_json(&output)?;
    } else {
        print_commands_tsv(&output.commands, &cfg)?;
    }

    Ok(())
}

async fn run_exec_command(cmd: ExecCommand) -> Result<()> {
    let ExecCommand {
        server,
        command_id,
        parameters,
        stream,
        quiet,
    } = cmd;

    let registry = ServerRegistry::load()?;
    let entry = find_enrolled_server(&server, &registry)?;

    let provided = parse_parameter_pairs(&parameters)?;
    let command_list = fetch_command_list(entry).await?;
    let command = command_list
        .commands
        .iter()
        .find(|c| c.id.eq_ignore_ascii_case(&command_id))
        .ok_or_else(|| anyhow!("Command '{}' not found on server {}", command_id, entry.id))?;

    let sanitized = validate_parameters(command, &provided)?;
    let actual_command_id = command.id.clone();

    match command.session_mode {
        CommandSessionMode::OneShot => {
            if command.kind != CommandKind::ShellScript {
                bail!(
                    "Capability '{}' has type {:?} which is not supported by this CLI",
                    command.name,
                    command.kind
                );
            }

            if command.requires_confirmation && !quiet {
                println!(
                    "Command '{}' requires confirmation on the server before execution",
                    command.name
                );
            }

            let exit_code = if stream {
                let mut on_event = |event: CommandStreamEvent| {
                    if quiet {
                        return;
                    }
                    print_stream_event(&event);
                };
                execute_command(entry, &actual_command_id, sanitized, &mut on_event).await?
            } else {
                let mut captured: Vec<CommandStreamEvent> = Vec::new();
                let mut capture = |event: CommandStreamEvent| {
                    if quiet {
                        return;
                    }
                    captured.push(event);
                };
                let exit =
                    execute_command(entry, &actual_command_id, sanitized, &mut capture).await?;

                if !quiet {
                    replay_buffered_events(&captured)?;
                }

                exit
            };

            if !quiet {
                if stream {
                    println!("EXIT {}", exit_code);
                } else {
                    println!("Exit code: {}", exit_code);
                }
            }

            if exit_code != 0 {
                bail!("Remote command exited with code {}", exit_code);
            }

            Ok(())
        }
        CommandSessionMode::Realtime => {
            if command.kind != CommandKind::ShellInteractive {
                bail!(
                    "Capability '{}' has type {:?} which is not supported by this CLI",
                    command.name,
                    command.kind
                );
            }
            if quiet {
                bail!("--quiet is not supported for realtime capabilities");
            }
            if command.requires_confirmation {
                println!(
                    "Capability '{}' requires confirmation on the server before execution",
                    command.name
                );
            }
            let session = open_capability_session(entry, &command.id, sanitized).await?;
            if session.session_mode() != CommandSessionMode::Realtime {
                bail!(
                    "Capability '{}' reported unsupported session mode {:?}",
                    command.name,
                    session.session_mode()
                );
            }
            let session_label = if command.name.is_empty() {
                command.id.clone()
            } else {
                command.name.clone()
            };
            run_interactive_shell(entry, &server, &session_label, session, false).await
        }
        _ => bail!(
            "Capability '{}' requires {:?} sessions which are not supported by this CLI",
            command.name,
            command.session_mode
        ),
    }
}

async fn run_interactive_shell(
    entry: &ServerRegistryEntry,
    server_label: &str,
    session_name: &str,
    mut session: CapabilitySession,
    resumed: bool,
) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("Realtime capabilities require an interactive TTY");
    }

    let config = config::load()?;
    let jiggle_resize = config.cli.jiggle_resize_on_resume;

    if resumed {
        println!("Resumed interactive session '{session_name}'.");
    } else if let Some(message) = session.ready_message() {
        if !message.is_empty() {
            println!("{message}");
        }
    } else {
        println!("Interactive session '{session_name}' ready.");
    }
    println!("Press Ctrl+] to detach.");

    let raw_guard = RawModeGuard::new()?;

    let mut sender = session.sender();
    send_initial_resize(&sender).await;

    let (mut input_handle, mut resize_handle, mut detach_rx) = setup_io_tasks(sender.clone());

    let mut stdout = tokio::io::stdout();
    let mut stderr = tokio::io::stderr();
    let mut exit_code: Option<i32> = None;
    let mut timed_out = false;
    let mut resume_attempts: usize = 0;
    maybe_jiggle(jiggle_resize, &mut sender).await?;
    let mut outcome = drive_session(
        &mut session,
        &mut stdout,
        &mut stderr,
        &mut detach_rx,
        &mut exit_code,
        &mut timed_out,
    )
    .await?;

    while matches!(outcome, SessionOutcome::StreamClosed) && exit_code.is_none() {
        let resume_state = session.resume_state();

        println!("\nConnection interrupted. Attempting to resume...");

        cleanup_io_tasks(&mut input_handle, &mut resize_handle).await;

        match resume_capability_session(entry, resume_state).await {
            Ok(new_session) => {
                session = new_session;
                sender = session.sender();
                send_initial_resize(&sender).await;
                maybe_jiggle(jiggle_resize, &mut sender).await?;

                let handles = setup_io_tasks(sender.clone());
                input_handle = handles.0;
                resize_handle = handles.1;
                detach_rx = handles.2;

                resume_attempts += 1;
                println!("Session resumed (attempt #{resume_attempts}).");

                outcome = drive_session(
                    &mut session,
                    &mut stdout,
                    &mut stderr,
                    &mut detach_rx,
                    &mut exit_code,
                    &mut timed_out,
                )
                .await?;
            }
            Err(err) => {
                cleanup_io_tasks(&mut input_handle, &mut resize_handle).await;
                drop(raw_guard);
                return Err(anyhow!("Failed to resume session: {}", err));
            }
        }
    }

    let detached = matches!(outcome, SessionOutcome::Detached);

    cleanup_io_tasks(&mut input_handle, &mut resize_handle).await;
    drop(raw_guard);

    if detached {
        let _ = sender.close(Some("Client detached".to_string())).await;
        println!("\nDetached from session {}.", session.session_id());
        println!(
            "Resume later with: handcontrol-cli resume {} {}",
            server_label,
            session.session_id()
        );
        println!(
            "You can view active sessions with: handcontrol-cli sessions {}",
            server_label
        );
        return Ok(());
    }

    let _ = sender.close(None).await;

    if timed_out {
        bail!("Realtime capability timed out");
    }

    let code = exit_code.ok_or_else(|| anyhow!("Realtime capability ended without exit code"))?;
    if code != 0 {
        bail!("Realtime capability exited with code {}", code);
    }

    Ok(())
}

async fn send_initial_resize(sender: &CapabilitySessionSender) {
    if let Ok((cols, rows)) = terminal_size() {
        if let Err(err) = sender.send_resize(u32::from(cols), u32::from(rows)).await {
            warn!("Failed to send terminal size: {}", err);
        }
    }
}

fn spawn_input_task(
    sender: CapabilitySessionSender,
    detach_tx: mpsc::UnboundedSender<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let mut buf = [0u8; 1024];
        loop {
            match stdin.read(&mut buf).await {
                Ok(0) => {
                    let _ = sender.close(None).await;
                    break;
                }
                Ok(n) => {
                    let start = 0;
                    let mut detach_triggered = false;

                    for (idx, byte) in buf[..n].iter().enumerate() {
                        if *byte == 0x1d {
                            if idx > start {
                                if sender
                                    .send_input(buf[start..idx].to_vec(), false)
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            let _ = detach_tx.send(());
                            detach_triggered = true;
                            break;
                        }
                    }

                    if detach_triggered {
                        return;
                    }

                    if start < n {
                        if sender
                            .send_input(buf[start..n].to_vec(), false)
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
                Err(_) => {
                    let _ = sender.close(Some("stdin read error".to_string())).await;
                    break;
                }
            }
        }
    })
}

fn spawn_resize_task(sender: CapabilitySessionSender) -> Option<tokio::task::JoinHandle<()>> {
    #[cfg(unix)]
    {
        Some(tokio::spawn(async move {
            if let Ok(mut sigwinch) = signal(SignalKind::window_change()) {
                while sigwinch.recv().await.is_some() {
                    if let Ok((cols, rows)) = terminal_size() {
                        if sender
                            .send_resize(u32::from(cols), u32::from(rows))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        }))
    }
    #[cfg(not(unix))]
    {
        let _ = sender;
        None
    }
}

fn setup_io_tasks(
    sender: CapabilitySessionSender,
) -> (
    tokio::task::JoinHandle<()>,
    Option<tokio::task::JoinHandle<()>>,
    mpsc::UnboundedReceiver<()>,
) {
    let (detach_tx, detach_rx) = mpsc::unbounded_channel();
    let input_handle = spawn_input_task(sender.clone(), detach_tx);
    let resize_handle = spawn_resize_task(sender);
    (input_handle, resize_handle, detach_rx)
}

enum SessionOutcome {
    Exit,
    StreamClosed,
    Detached,
}

async fn drive_session(
    session: &mut CapabilitySession,
    stdout: &mut tokio::io::Stdout,
    stderr: &mut tokio::io::Stderr,
    detach_rx: &mut mpsc::UnboundedReceiver<()>,
    exit_code: &mut Option<i32>,
    timed_out: &mut bool,
) -> Result<SessionOutcome> {
    loop {
        tokio::select! {
            Some(_) = detach_rx.recv() => return Ok(SessionOutcome::Detached),
            event = session.recv() => match event {
                Ok(Some(CapabilitySessionEvent::Output { data, stderr: is_stderr, .. })) => {
                    let write_result = if is_stderr {
                        stderr.write_all(&data).await
                    } else {
                        stdout.write_all(&data).await
                    };

                    if write_result.is_err() {
                        return Ok(SessionOutcome::StreamClosed);
                    }

                    let _ = if is_stderr {
                        stderr.flush().await
                    } else {
                        stdout.flush().await
                    };
                }
                Ok(Some(CapabilitySessionEvent::Exit { exit_code: code, timed_out: was_timeout, message })) => {
                    *exit_code = Some(code);
                    *timed_out = was_timeout;
                    if let Some(message) = message {
                        let mut bytes = message.into_bytes();
                        if !bytes.ends_with(&[b'\n']) {
                            bytes.push(b'\n');
                        }
                        let _ = stderr.write_all(&bytes).await;
                        let _ = stderr.flush().await;
                    }
                    return Ok(SessionOutcome::Exit);
                }
                Ok(Some(CapabilitySessionEvent::Error { message, .. })) => {
                    return Err(anyhow!("Server reported capability error: {}", message));
                }
                Ok(Some(CapabilitySessionEvent::HeartbeatAck { .. })) => {
                    // keep alive
                }
                Ok(Some(CapabilitySessionEvent::Closed { reason })) => {
                    if exit_code.is_none() {
                        if let Some(reason) = reason {
                            return Err(anyhow!("Capability session closed: {}", reason));
                        } else {
                            return Err(anyhow!("Capability session closed unexpectedly"));
                        }
                    }
                    return Ok(SessionOutcome::StreamClosed);
                }
                Ok(None) => return Ok(SessionOutcome::StreamClosed),
                Err(err) => return Err(err),
            },
        }
    }
}

async fn cleanup_io_tasks(
    input_handle: &mut tokio::task::JoinHandle<()>,
    resize_handle: &mut Option<tokio::task::JoinHandle<()>>,
) {
    input_handle.abort();
    let _ = input_handle.await;

    if let Some(handle) = resize_handle.take() {
        handle.abort();
        let _ = handle.await;
    }
}

async fn maybe_jiggle(enabled: bool, sender: &mut CapabilitySessionSender) -> Result<()> {
    if enabled {
        jiggle_terminal(sender).await?;
    }
    Ok(())
}

async fn jiggle_terminal(sender: &mut CapabilitySessionSender) -> Result<()> {
    if let Ok((cols, rows)) = terminal_size() {
        if cols == 0 || rows == 0 {
            return Ok(());
        }

        sender
            .send_resize(u32::from(cols.saturating_add(1)), u32::from(rows))
            .await
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        sender
            .send_resize(u32::from(cols), u32::from(rows))
            .await
            .ok();
    }

    Ok(())
}

async fn run_sessions(cmd: SessionsCommand) -> Result<()> {
    let registry = ServerRegistry::load()?;
    let entry = find_enrolled_server(&cmd.server, &registry)?.clone();

    let mut sessions = fetch_sessions(&entry).await?;
    sessions.sort_by(|a, b| a.created_at_ms.cmp(&b.created_at_ms));

    if cmd.json {
        output_json(&sessions)?;
        return Ok(());
    }

    print_sessions_table(&sessions);
    Ok(())
}

async fn run_resume(cmd: ResumeCommand) -> Result<()> {
    let registry = ServerRegistry::load()?;
    let entry = find_enrolled_server(&cmd.server, &registry)?.clone();

    let sessions = fetch_sessions(&entry).await?;
    let mut info_matches: Vec<ServerSessionInfo> = Vec::new();
    let normalized = cmd.session_id.to_ascii_lowercase();
    let compact: String = normalized.chars().filter(|c| *c != '-').collect();

    for session in sessions {
        if session.session_id.eq_ignore_ascii_case(&cmd.session_id) {
            info_matches.clear();
            info_matches.push(session);
            break;
        }

        if normalized.is_empty() {
            continue;
        }

        let canonical = session.session_id.to_ascii_lowercase();
        let simple: String = canonical.chars().filter(|c| *c != '-').collect();
        if canonical.starts_with(&normalized)
            || (!compact.is_empty() && simple.starts_with(&compact))
        {
            info_matches.push(session);
        }
    }

    let info = match info_matches.len() {
        0 => {
            bail!(
                "Session '{}' not found on server {}",
                cmd.session_id,
                entry.id
            )
        }
        1 => info_matches.pop().unwrap(),
        _ => {
            let candidates = info_matches
                .iter()
                .map(|session| session.session_id.clone())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "Session identifier '{}' is ambiguous on server {}; matches: {}",
                cmd.session_id,
                entry.id,
                candidates
            );
        }
    };

    if info.session_mode != CommandSessionMode::Realtime {
        bail!(
            "Session '{}' uses '{:?}' mode which cannot be resumed interactively",
            info.session_id,
            info.session_mode
        );
    }

    let state = CapabilitySessionResumeState {
        session_id: info.session_id.clone(),
        capability_id: info.capability_id.clone(),
        resume_token: info.resume_token.clone(),
        last_stdout_sequence: info.stdout_next_sequence.saturating_sub(1),
        last_stderr_sequence: info.stderr_next_sequence.saturating_sub(1),
        session_mode: info.session_mode,
    };

    let session = resume_capability_session(&entry, state).await?;
    let session_label = if info.capability_name.is_empty() {
        info.capability_id
    } else {
        info.capability_name
    };

    run_interactive_shell(&entry, &cmd.server, &session_label, session, true).await
}

async fn run_completions(cmd: CompletionCommand) -> Result<()> {
    match cmd.command {
        CompletionSubcommand::Generate(args) => generate_completion_script(args),
        CompletionSubcommand::Dynamic(cmd) => run_dynamic_completion(cmd).await,
    }
}

fn generate_completion_script(args: GenerateCompletionCommand) -> Result<()> {
    let GenerateCompletionCommand { shell, output } = args;
    let mut command = Cli::command();
    let bin_name = command.get_name().to_string();

    let mut buffer: Vec<u8> = Vec::new();
    generate(shell, &mut command, bin_name.as_str(), &mut buffer);
    let mut script =
        String::from_utf8(buffer).context("Generated completion script was not UTF-8")?;

    match shell {
        Shell::Bash => enhance_bash_completion(&mut script),
        Shell::Zsh => enhance_zsh_completion(&mut script),
        Shell::Fish => enhance_fish_completion(&mut script),
        _ => {}
    }

    if let Some(path) = output {
        let mut file = fs::File::create(&path)
            .with_context(|| format!("Failed to create completion file {}", path.display()))?;
        file.write_all(script.as_bytes())
            .with_context(|| format!("Failed to write completion file {}", path.display()))?;
    } else {
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        handle.write_all(script.as_bytes())?;
        handle.flush()?;
    }

    Ok(())
}

async fn run_dynamic_completion(cmd: DynamicCompletionCommand) -> Result<()> {
    match cmd.target {
        DynamicCompletionTarget::Servers { prefix } => {
            let registry = ServerRegistry::load()?;
            let prefix = prefix.unwrap_or_default();
            for candidate in collect_server_candidates(&prefix, &registry) {
                println!("{candidate}");
            }
            Ok(())
        }
        DynamicCompletionTarget::Sessions { server, prefix } => {
            let registry = ServerRegistry::load()?;
            let entry = find_enrolled_server(&server, &registry)?.clone();
            let sessions = fetch_sessions(&entry).await?;
            let prefix = prefix.unwrap_or_default();
            for candidate in collect_session_candidates(&sessions, &prefix) {
                println!("{candidate}");
            }
            Ok(())
        }
    }
}

fn print_sessions_table(sessions: &[ServerSessionInfo]) {
    if sessions.is_empty() {
        println!("No active sessions.");
        return;
    }

    println!(
        "{:<36}  {:<24}  {:<8}  {:<7}  {:<10}  {:<12}  {}",
        "Session ID", "Capability", "Mode", "Attached", "Owner", "Age", "Last Activity"
    );

    for info in sessions {
        let capability_display = if info.capability_name.is_empty() {
            info.capability_id.clone()
        } else {
            info.capability_name.clone()
        };

        println!(
            "{:<36}  {:<24}  {:<8}  {:<7}  {:<10}  {:<12}  {}",
            info.session_id,
            truncate(&capability_display, 24),
            format!("{:?}", info.session_mode),
            if info.attached { "yes" } else { "no" },
            short_fingerprint(&info.owner_fingerprint),
            format_age(info.created_at_ms),
            format_since(info.last_activity_ms),
        );
    }
}

fn truncate(value: &str, width: usize) -> String {
    if value.len() <= width {
        value.to_string()
    } else if width == 0 {
        String::new()
    } else if width == 1 {
        "…".to_string()
    } else {
        format!("{}…", &value[..width - 1])
    }
}

fn short_fingerprint(fingerprint: &Option<String>) -> String {
    match fingerprint {
        Some(fp) if !fp.is_empty() => {
            if fp.len() <= 8 {
                fp.clone()
            } else {
                format!("{}…", &fp[..8])
            }
        }
        _ => "-".to_string(),
    }
}

fn timestamp_to_datetime(ms: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos((ms as i128) * 1_000_000).ok()
}

fn format_age(ms: i64) -> String {
    match timestamp_to_datetime(ms) {
        Some(created) => {
            let now = OffsetDateTime::now_utc();
            if now >= created {
                duration_to_brief(now - created)
            } else {
                "0s".to_string()
            }
        }
        None => "n/a".to_string(),
    }
}

fn format_since(ms: i64) -> String {
    match timestamp_to_datetime(ms) {
        Some(timestamp) => {
            let now = OffsetDateTime::now_utc();
            if now >= timestamp {
                format!("{} ago", duration_to_brief(now - timestamp))
            } else {
                "in future".to_string()
            }
        }
        None => "n/a".to_string(),
    }
}

fn duration_to_brief(duration: TimeDuration) -> String {
    let mut seconds = duration.whole_seconds();
    if seconds <= 0 {
        return "0s".to_string();
    }

    let mut parts = Vec::new();
    let days = seconds / 86_400;
    if days > 0 {
        parts.push(format!("{}d", days));
        seconds -= days * 86_400;
    }

    let hours = seconds / 3_600;
    if hours > 0 {
        parts.push(format!("{}h", hours));
        seconds -= hours * 3_600;
    }

    let minutes = seconds / 60;
    if minutes > 0 && parts.len() < 2 {
        parts.push(format!("{}m", minutes));
        seconds -= minutes * 60;
    }

    if seconds > 0 && parts.len() < 2 {
        parts.push(format!("{}s", seconds));
    }

    if parts.is_empty() {
        "0s".to_string()
    } else {
        parts.join(" ")
    }
}

fn run_remove(cmd: RemoveCommand) -> Result<()> {
    let RemoveCommand { server, confirm } = cmd;

    let mut registry = ServerRegistry::load()?;
    if registry.iter().next().is_none() {
        bail!("No enrolled servers to remove");
    }

    let entry = find_enrolled_server(&server, &registry)?.clone();

    if !confirm && !prompt_removal_confirmation(&entry)? {
        println!("Aborted");
        return Ok(());
    }

    let server_id = entry.id;
    registry.remove(&server_id);
    registry.save()?;

    if let Ok(cert_dir) = certificate_dir_for(&entry) {
        if cert_dir.exists() {
            match fs::remove_dir_all(&cert_dir) {
                Ok(_) => {
                    println!("Removed credentials in {}", cert_dir.display());
                }
                Err(err) => {
                    eprintln!(
                        "Warning: failed to remove credential directory {} ({err})",
                        cert_dir.display()
                    );
                }
            }
        }
    }

    println!("Removed enrollment for server {}", server_id);
    Ok(())
}

fn run_config(command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Show { json } => {
            let cfg = config::load()?;
            if json {
                output_json(&cfg)?;
            } else {
                let toml = toml::to_string_pretty(&cfg)?;
                println!("{toml}");
            }
        }
        ConfigCommand::Path => {
            let path = config::config_path()?;
            println!("{}", path.display());
        }
        ConfigCommand::Set { key, value } => {
            let mut cfg = config::load()?;
            set_config_value(&mut cfg, &key, &value)?;
            config::save(&cfg)?;
            println!("Updated {key}");
        }
    }
    Ok(())
}

async fn run_enroll(command: EnrollCommand) -> Result<()> {
    match command {
        EnrollCommand::Qr(args) => run_enroll_qr(args).await,
        EnrollCommand::Approve(args) => run_enroll_approve(args).await,
    }
}

fn extract_token_expiry(payload: &str) -> Result<Option<OffsetDateTime>> {
    let value: Value = serde_json::from_str(payload)?;
    if let Some(valid_until_value) = value.get("valid_until") {
        let raw = valid_until_value
            .as_str()
            .ok_or_else(|| anyhow!("valid_until must be a string"))?;
        let expiry = OffsetDateTime::parse(raw, &Rfc3339)
            .context("Invalid valid_until timestamp in payload")?;
        Ok(Some(expiry))
    } else {
        Ok(None)
    }
}

fn format_remaining(duration: TimeDuration) -> String {
    let total_seconds = duration.whole_seconds();
    if total_seconds <= 0 {
        return "0s".to_string();
    }
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    if minutes > 0 {
        format!("{}m {:02}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}

async fn run_enroll_qr(args: QrEnrollCommand) -> Result<()> {
    let QrEnrollCommand {
        payload,
        payload_file,
        server_id,
        device_name,
        device_model,
    } = args;

    let raw_payload = match (payload, payload_file) {
        (Some(raw), None) => raw,
        (None, Some(path)) => fs::read_to_string(&path)
            .with_context(|| format!("Failed to read payload file {}", path.display()))?,
        (None, None) => read_payload_from_stdin()?,
        _ => unreachable!("clap enforces mutual exclusivity"),
    }
    .trim()
    .to_string();

    if let Some(expiry) = extract_token_expiry(&raw_payload)? {
        let now = OffsetDateTime::now_utc();
        let formatted_expiry = expiry
            .format(&Rfc3339)
            .unwrap_or_else(|_| expiry.to_string());
        if expiry <= now {
            bail!("Enrollment QR code expired at {}", formatted_expiry);
        } else {
            let remaining = expiry - now;
            println!(
                "QR token expires in {} ({} UTC)",
                format_remaining(remaining),
                formatted_expiry
            );
        }
    }

    let override_server_id = if let Some(id) = server_id {
        Some(Uuid::parse_str(&id).context("Invalid UUID passed to --server-id")?)
    } else {
        None
    };

    let cfg = config::load()?;
    let resolved_device_name =
        device_name.or_else(|| cfg.device.as_ref().and_then(|d| d.name.clone()));
    let resolved_device_model =
        device_model.or_else(|| cfg.device.as_ref().and_then(|d| d.model.clone()));

    let outcome = enroll_via_qr(QrEnrollmentInput {
        payload: raw_payload,
        override_server_id,
        device_name: resolved_device_name,
        device_model: resolved_device_model,
    })
    .await?;

    println!("Successfully enrolled to server {}", outcome.server_id);
    if let Some(client_id) = outcome.client_id {
        println!("Client ID: {client_id}");
    }
    if !outcome.addresses.is_empty() {
        if outcome.addresses.len() == 1 {
            println!("Direct address: {}:{}", outcome.addresses[0], outcome.port);
        } else {
            println!("Direct addresses (port {}):", outcome.port);
            for addr in &outcome.addresses {
                println!("  - {}:{}", addr, outcome.port);
            }
        }
    } else {
        println!("No direct addresses recorded.");
    }
    if let Some(relay) = &outcome.relay {
        println!(
            "Relay URL: {} (required: {})",
            relay.relay_url,
            if relay.relay_required { "yes" } else { "no" }
        );
        if relay.allow_self_signed_tls {
            println!("Relay TLS: allowing self-signed certificates");
        }
    }
    println!("Credentials stored in {}", outcome.cert_directory.display());
    if let Some(expiry) = outcome.token_expiry {
        let now = OffsetDateTime::now_utc();
        let formatted_expiry = expiry
            .format(&Rfc3339)
            .unwrap_or_else(|_| expiry.to_string());
        if expiry > now {
            println!(
                "Original QR token expires in {} ({} UTC)",
                format_remaining(expiry - now),
                formatted_expiry
            );
        } else {
            println!("Original QR token expired at {}", formatted_expiry);
        }
    }

    Ok(())
}

async fn run_enroll_approve(args: ApproveEnrollCommand) -> Result<()> {
    let ApproveEnrollCommand {
        server,
        address,
        port,
        timeout,
        poll_interval,
        device_name,
        device_model,
    } = args;

    let cfg = config::load()?;
    let resolved = resolve_server_targets(&server, address, port, &cfg)?;

    let resolved_device_name =
        device_name.or_else(|| cfg.device.as_ref().and_then(|d| d.name.clone()));
    let resolved_device_model =
        device_model.or_else(|| cfg.device.as_ref().and_then(|d| d.model.clone()));

    let input = ApprovalEnrollmentInput {
        addresses: resolved.addresses,
        port: resolved.port,
        server_id_hint: resolved.server_id_hint,
        device_name: resolved_device_name,
        device_model: resolved_device_model,
        timeout: Duration::from_secs(timeout),
        poll_interval: Duration::from_secs(poll_interval.max(1)),
    };

    let mut last_code: Option<String> = None;
    let outcome = enroll_via_approval(input, |code| {
        last_code = Some(code.to_string());
        print_verification_block(code);
    })
    .await?;

    println!("Enrollment approved for server {}", outcome.server_id);
    if let Some(client_id) = outcome.client_id {
        println!("Client ID: {client_id}");
    }
    if !outcome.addresses.is_empty() {
        if outcome.addresses.len() == 1 {
            println!("Direct address: {}:{}", outcome.addresses[0], outcome.port);
        } else {
            println!("Direct addresses (port {}):", outcome.port);
            for addr in &outcome.addresses {
                println!("  - {}:{}", addr, outcome.port);
            }
        }
    } else {
        println!("No direct addresses recorded.");
    }
    if let Some(relay) = &outcome.relay {
        println!(
            "Relay URL: {} (required: {})",
            relay.relay_url,
            if relay.relay_required { "yes" } else { "no" }
        );
        if relay.allow_self_signed_tls {
            println!("Relay TLS: allowing self-signed certificates");
        }
    }
    println!("Credentials stored in {}", outcome.cert_directory.display());
    if let Some(code) = last_code {
        println!("Verification code confirmed: {}", code);
    }

    Ok(())
}

fn print_verification_block(code: &str) {
    println!();
    println!("Verification code");
    let inner_width = code.len().max(7);
    let border = format!("+{}+", "-".repeat(inner_width + 4));
    println!("{border}");
    println!("|  {:^width$}  |", code, width = inner_width);
    println!("{border}");
    println!("Approve this request on the server to continue...");
    println!();
}

#[derive(Debug, Serialize)]
struct InfoOutput {
    server_id: Uuid,
    hostname: Option<String>,
    version: Option<String>,
    os: Option<String>,
    address: String,
    tls_fingerprint: String,
    enrolled: bool,
    client_id: Option<Uuid>,
}

fn print_info_output(info: &InfoOutput) {
    println!("Server ID: {}", info.server_id);
    println!("Hostname: {}", info.hostname.as_deref().unwrap_or("-"));
    println!("Version: {}", info.version.as_deref().unwrap_or("-"));
    println!("OS: {}", info.os.as_deref().unwrap_or("-"));
    println!("Address: {}", info.address);
    println!("TLS Fingerprint: {}", info.tls_fingerprint);
    println!("Enrolled: {}", if info.enrolled { "yes" } else { "no" });
    println!(
        "Client ID: {}",
        info.client_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
}

fn format_socket_address(address: &str, port: u16) -> String {
    if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    }
}

struct ResolvedServer {
    addresses: Vec<String>,
    port: Option<u16>,
    server_id_hint: Option<Uuid>,
}

fn resolve_server_targets(
    server: &str,
    address: Option<String>,
    port: Option<u16>,
    cfg: &ClientConfig,
) -> Result<ResolvedServer> {
    let mut addresses: Vec<String> = Vec::new();
    let mut resolved_port = port;
    let mut server_id_hint = Uuid::parse_str(server).ok();

    if let Some(addr) = address {
        if let Some((host, parsed_port)) = parse_host_port(&addr) {
            addresses.push(host);
            if resolved_port.is_none() {
                resolved_port = Some(parsed_port);
            }
        } else {
            addresses.push(addr);
        }
    } else {
        let discovered = discover_servers(&cfg.discovery)?;
        let query = server.to_ascii_lowercase();
        let compact_query: String = query.chars().filter(|c| *c != '-').collect();

        let mut host_matches: Vec<DiscoveredServer> = Vec::new();
        let mut id_matches: Vec<DiscoveredServer> = Vec::new();

        for entry in discovered {
            if entry.instance_name.eq_ignore_ascii_case(server)
                || entry.hostname.eq_ignore_ascii_case(server)
            {
                if server_id_hint.is_none() {
                    server_id_hint = entry.server_id;
                }
                host_matches.push(entry);
                continue;
            }

            if let Some(id) = entry.server_id {
                let canonical_lower = id.to_string().to_ascii_lowercase();
                let simple_lower = id.simple().to_string().to_ascii_lowercase();
                if canonical_lower == query
                    || canonical_lower.starts_with(&query)
                    || (!compact_query.is_empty() && simple_lower.starts_with(&compact_query))
                {
                    if server_id_hint.is_none() {
                        server_id_hint = Some(id);
                    }
                    id_matches.push(entry);
                }
            }
        }

        let mut matches = if !host_matches.is_empty() {
            host_matches
        } else {
            id_matches
        };

        if matches.is_empty() {
            bail!(
                "Unable to resolve server '{server}'. Run 'handcontrol-cli discover' or supply --address"
            );
        }

        if matches.len() > 1 {
            let descriptions = matches
                .iter()
                .map(|entry| {
                    entry
                        .server_id
                        .map(|id| id.to_string())
                        .or_else(|| {
                            if !entry.instance_name.is_empty() {
                                Some(entry.instance_name.clone())
                            } else if !entry.hostname.is_empty() {
                                Some(entry.hostname.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| "<unknown>".to_string())
                })
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "Server identifier '{}' is ambiguous; matches: {}",
                server,
                descriptions
            );
        }

        let entry = matches.pop().unwrap();
        let entry_port = entry.port;
        let entry_server_id = entry.server_id;
        let entry_addresses = entry.addresses;

        if resolved_port.is_none() {
            resolved_port = Some(entry_port);
        }
        if server_id_hint.is_none() {
            server_id_hint = entry_server_id;
        }
        addresses.extend(entry_addresses);
    }

    addresses.sort();
    addresses.dedup();

    if addresses.is_empty() {
        bail!(
            "Unable to resolve server '{server}'. Run 'handcontrol-cli discover' or supply --address"
        );
    }

    Ok(ResolvedServer {
        addresses,
        port: resolved_port,
        server_id_hint,
    })
}

fn prompt_removal_confirmation(entry: &ServerRegistryEntry) -> Result<bool> {
    let display_name = entry
        .hostname
        .as_deref()
        .or(entry.ip.as_deref())
        .unwrap_or("unknown");

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    write!(
        handle,
        "Remove enrollment for {} ({display_name})? [y/N]: ",
        entry.id
    )?;
    handle.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let normalized = input.trim().to_ascii_lowercase();
    Ok(matches!(normalized.as_str(), "y" | "yes"))
}

fn certificate_dir_for(entry: &ServerRegistryEntry) -> Result<PathBuf> {
    if let Some(path) = &entry.cert_path {
        return Ok(PathBuf::from(path));
    }

    let mut dir = config::config_dir()?;
    dir.push("client-certs");
    dir.push(entry.id.to_string());
    Ok(dir)
}

#[derive(Debug, Serialize)]
struct CommandsOutput {
    server_id: Uuid,
    config_version: u64,
    commands: Vec<CommandSummary>,
}

fn print_commands_tsv(commands: &[CommandSummary], cfg: &ClientConfig) -> Result<()> {
    let mut stdout = io::BufWriter::new(io::stdout().lock());

    if cfg.cli.show_headers {
        writeln!(stdout, "ID\tNAME\tDESCRIPTION\tTAGS\tKIND\tSESSION_MODE")?;
    }

    for command in commands {
        let tags = if command.tags.is_empty() {
            "-".to_string()
        } else {
            command.tags.join(",")
        };
        writeln!(
            stdout,
            "{}\t{}\t{}\t{}\t{}\t{}",
            command.id,
            command.name,
            command.description.as_deref().unwrap_or("-"),
            tags,
            format!("{:?}", command.kind).to_ascii_lowercase(),
            format!("{:?}", command.session_mode).to_ascii_lowercase()
        )?;
    }

    stdout.flush()?;
    Ok(())
}

fn replay_buffered_events(events: &[CommandStreamEvent]) -> Result<()> {
    let stdout = io::stdout();
    let stderr = io::stderr();
    let mut out = stdout.lock();
    let mut err = stderr.lock();

    for event in events {
        match event {
            CommandStreamEvent::Stdout(data) => {
                out.write_all(data.as_bytes())?;
                out.flush()?;
            }
            CommandStreamEvent::Stderr(data) => {
                err.write_all(data.as_bytes())?;
                err.flush()?;
            }
        }
    }

    Ok(())
}

fn print_stream_event(event: &CommandStreamEvent) {
    match event {
        CommandStreamEvent::Stdout(data) => print_prefixed_chunk("STDOUT", data),
        CommandStreamEvent::Stderr(data) => print_prefixed_chunk("STDERR", data),
    }
}

fn print_prefixed_chunk(prefix: &str, data: &str) {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    for chunk in data.split_inclusive('\n') {
        if chunk.is_empty() {
            continue;
        }
        if chunk.ends_with('\n') {
            let _ = write!(handle, "{} {}", prefix, chunk);
        } else {
            let _ = writeln!(handle, "{} {}", prefix, chunk);
        }
    }
    let _ = handle.flush();
}

struct RawModeGuard;

impl RawModeGuard {
    fn new() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw terminal mode")?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if let Err(err) = disable_raw_mode() {
            warn!("Failed to restore terminal state: {}", err);
        }
    }
}

fn parse_parameter_pairs(values: &[String]) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for pair in values {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| anyhow!("Parameter '{pair}' must be in key=value format"))?;
        let key = key.trim();
        if key.is_empty() {
            bail!("Parameter key cannot be empty in '{pair}'");
        }
        let value = value.to_string();
        if map.insert(key.to_string(), value).is_some() {
            bail!("Duplicate parameter '{}' provided", key);
        }
    }
    Ok(map)
}

fn find_enrolled_server<'a>(
    identifier: &str,
    registry: &'a ServerRegistry,
) -> Result<&'a ServerRegistryEntry> {
    if let Ok(id) = Uuid::parse_str(identifier) {
        if let Some(entry) = registry.find_by_id(&id) {
            return Ok(entry);
        }
    }

    for entry in registry.iter() {
        if entry
            .hostname
            .as_ref()
            .map(|h| h.eq_ignore_ascii_case(identifier))
            .unwrap_or(false)
            || entry
                .ip
                .as_ref()
                .map(|ip| ip.eq_ignore_ascii_case(identifier))
                .unwrap_or(false)
            || entry
                .addresses
                .iter()
                .any(|addr| addr.eq_ignore_ascii_case(identifier))
        {
            return Ok(entry);
        }
    }

    let normalized = identifier.to_ascii_lowercase();
    if !normalized.is_empty() {
        let compact: String = normalized.chars().filter(|c| *c != '-').collect();
        let matches: Vec<&ServerRegistryEntry> = registry
            .iter()
            .filter(|entry| {
                let canonical = entry.id.to_string().to_ascii_lowercase();
                let simple = entry.id.simple().to_string().to_ascii_lowercase();
                canonical.starts_with(&normalized)
                    || (!compact.is_empty() && simple.starts_with(&compact))
            })
            .collect();

        match matches.len() {
            0 => {}
            1 => return Ok(matches[0]),
            _ => {
                let candidates = matches
                    .iter()
                    .map(|entry| entry.id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!(
                    "Server identifier '{}' is ambiguous. Matching IDs: {}",
                    identifier,
                    candidates
                );
            }
        }
    }

    bail!(
        "Server '{}' is not enrolled. Run 'handcontrol-cli enroll' to register it first.",
        identifier
    );
}

fn collect_server_candidates(prefix: &str, registry: &ServerRegistry) -> Vec<String> {
    let normalized = prefix.to_ascii_lowercase();
    let compact: String = normalized.chars().filter(|c| *c != '-').collect();
    let mut candidates: HashSet<String> = HashSet::new();

    for entry in registry.iter() {
        let id_str = entry.id.to_string();
        let id_lower = id_str.to_ascii_lowercase();
        let simple_lower = entry.id.simple().to_string().to_ascii_lowercase();
        let id_matches = normalized.is_empty()
            || id_lower.starts_with(&normalized)
            || (!compact.is_empty() && simple_lower.starts_with(&compact));
        if id_matches {
            candidates.insert(id_str);
        }

        if let Some(host) = &entry.hostname {
            if normalized.is_empty() || host.to_ascii_lowercase().starts_with(&normalized) {
                candidates.insert(host.clone());
            }
        }

        if let Some(ip) = &entry.ip {
            if normalized.is_empty() || ip.to_ascii_lowercase().starts_with(&normalized) {
                candidates.insert(ip.clone());
            }
        }

        for address in &entry.addresses {
            if normalized.is_empty() || address.to_ascii_lowercase().starts_with(&normalized) {
                candidates.insert(address.clone());
            }
        }
    }

    let mut results: Vec<String> = candidates.into_iter().collect();
    results.sort();
    results
}

fn collect_session_candidates(sessions: &[ServerSessionInfo], prefix: &str) -> Vec<String> {
    let normalized = prefix.to_ascii_lowercase();
    let compact: String = normalized.chars().filter(|c| *c != '-').collect();
    let mut results: Vec<String> = sessions
        .iter()
        .filter_map(|session| {
            let id_lower = session.session_id.to_ascii_lowercase();
            let simple_lower: String = id_lower.chars().filter(|c| *c != '-').collect();
            if normalized.is_empty()
                || id_lower.starts_with(&normalized)
                || (!compact.is_empty() && simple_lower.starts_with(&compact))
            {
                Some(session.session_id.clone())
            } else {
                None
            }
        })
        .collect();

    results.sort();
    results.dedup();
    results
}

fn enhance_bash_completion(script: &mut String) {
    if let Some(idx) = script.find("\n# HandControl dynamic completion helpers") {
        script.truncate(idx);
    }

    if !script.ends_with('\n') {
        script.push('\n');
    }

    script.push_str(
        r#"
# HandControl dynamic completion helpers
__handcontrol_cli_collect_servers() {
    local prefix="${1-}"
    HANDCONTROL_CLI_SUPPRESS_COMPLETION=1 handcontrol-cli completions dynamic servers "${prefix}"
}

__handcontrol_cli_collect_sessions() {
    local server="$1"
    local prefix="${2-}"
    if [[ -z "$server" ]]; then
        return
    fi
    HANDCONTROL_CLI_SUPPRESS_COMPLETION=1 handcontrol-cli completions dynamic sessions "${server}" "${prefix}"
}

__handcontrol_cli_complete_dynamic() {
    local cur prev
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev=""
    if (( COMP_CWORD > 0 )); then
        prev="${COMP_WORDS[COMP_CWORD-1]}"
    fi

    if [[ "$cur" == -* || "$prev" == -* ]]; then
        _handcontrol-cli
        return
    fi

    local primary=""
    local secondary=""
    local server_arg=""
    local positional_after_primary=0
    for ((i=1; i<COMP_CWORD; ++i)); do
        local word="${COMP_WORDS[i]}"
        if [[ "$word" == -* ]]; then
            continue
        fi
        if [[ -z "$primary" ]]; then
            primary="$word"
            continue
        fi
        if [[ "$primary" == "enroll" && -z "$secondary" ]]; then
            secondary="$word"
            continue
        fi
        if [[ -z "$server_arg" ]]; then
            server_arg="$word"
            positional_after_primary=1
            continue
        fi
        positional_after_primary=2
        break
    done

    case "$primary" in
        info|list|exec|sessions|remove)
            if [[ -z "$server_arg" ]]; then
                local suggestions
                suggestions=$(__handcontrol_cli_collect_servers "$cur" 2>/dev/null)
                if [[ -n "$suggestions" ]]; then
                    COMPREPLY=($(compgen -W "$suggestions" -- "$cur"))
                    return
                fi
            fi
            ;;
        resume)
            if [[ -z "$server_arg" ]]; then
                local suggestions
                suggestions=$(__handcontrol_cli_collect_servers "$cur" 2>/dev/null)
                if [[ -n "$suggestions" ]]; then
                    COMPREPLY=($(compgen -W "$suggestions" -- "$cur"))
                    return
                fi
            elif (( positional_after_primary == 1 )); then
                local server="$server_arg"
                local suggestions
                suggestions=$(__handcontrol_cli_collect_sessions "$server" "$cur" 2>/dev/null)
                if [[ -n "$suggestions" ]]; then
                    COMPREPLY=($(compgen -W "$suggestions" -- "$cur"))
                    return
                fi
            fi
            ;;
        enroll)
            if [[ "$secondary" == "approve" && -z "$server_arg" ]]; then
                local suggestions
                suggestions=$(__handcontrol_cli_collect_servers "$cur" 2>/dev/null)
                if [[ -n "$suggestions" ]]; then
                    COMPREPLY=($(compgen -W "$suggestions" -- "$cur"))
                    return
                fi
            fi
            ;;
    esac

    _handcontrol-cli
}
complete -F __handcontrol_cli_complete_dynamic -o nosort -o bashdefault -o default handcontrol-cli
"#,
    );
}

fn enhance_zsh_completion(script: &mut String) {
    if let Some(idx) = script.find("\n_handcontrol_cli_servers()") {
        script.truncate(idx);
    }

    if !script.ends_with('\n') {
        script.push('\n');
    }

    let server_patterns = [
        ":server -- Server identifier (UUID, instance name, or hostname):_default",
        ":server -- Server identifier (UUID, hostname, or alias):_default",
    ];

    for pattern in server_patterns {
        if script.contains(pattern) {
            let replacement = pattern.replace("_default", "_handcontrol_cli_servers");
            *script = script.replace(pattern, &replacement);
        }
    }

    let session_pattern = ":session-id -- Session identifier (UUID):_default";
    if script.contains(session_pattern) {
        *script = script.replace(
            session_pattern,
            ":session-id -- Session identifier (UUID):_handcontrol_cli_sessions",
        );
    }

    script.push_str(
        r#"
_handcontrol_cli_servers() {
    local -a suggestions
    local prefix="$words[CURRENT]"
    suggestions=(${(f)"$(HANDCONTROL_CLI_SUPPRESS_COMPLETION=1 handcontrol-cli completions dynamic servers \"$prefix\" 2>/dev/null)"})
    compadd -Q -- "${suggestions[@]}"
}

_handcontrol_cli_sessions() {
    local server="$words[3]"
    local prefix="$words[CURRENT]"
    if [[ -z "$server" ]]; then
        return
    fi
    local -a suggestions
    suggestions=(${(f)"$(HANDCONTROL_CLI_SUPPRESS_COMPLETION=1 handcontrol-cli completions dynamic sessions \"$server\" \"$prefix\" 2>/dev/null)"})
    compadd -Q -- "${suggestions[@]}"
}
"#,
    );
}

fn enhance_fish_completion(_script: &mut String) {}

fn set_config_value(cfg: &mut ClientConfig, key: &str, value: &str) -> Result<()> {
    let parts: Vec<&str> = key.split('.').collect();
    match parts.as_slice() {
        ["discovery", "auto_discover"] => {
            cfg.discovery.auto_discover = parse_bool_arg(value)?;
        }
        ["discovery", "timeout_seconds"] => {
            cfg.discovery.timeout_seconds = parse_u64_arg(key, value)?;
        }
        ["discovery", "prefer_ipv6"] => {
            cfg.discovery.prefer_ipv6 = parse_bool_arg(value)?;
        }
        ["discovery", "include_link_local"] => {
            cfg.discovery.include_link_local = parse_bool_arg(value)?;
        }
        ["connection", "timeout_seconds"] => {
            cfg.connection.timeout_seconds = parse_u64_arg(key, value)?;
        }
        ["connection", "command_timeout_seconds"] => {
            cfg.connection.command_timeout_seconds = parse_u64_arg(key, value)?;
        }
        ["connection", "retry_attempts"] => {
            cfg.connection.retry_attempts = parse_u32_arg(key, value)?;
        }
        ["connection", "retry_delay_ms"] => {
            cfg.connection.retry_delay_ms = parse_u64_arg(key, value)?;
        }
        ["network", "relay", "prefer_relay"] => {
            cfg.network.relay.prefer_relay = parse_bool_arg(value)?;
        }
        ["network", "relay", "relay_only_mode"] => {
            cfg.network.relay.relay_only_mode = parse_bool_arg(value)?;
        }
        ["network", "relay", "max_direct_attempts"] => {
            cfg.network.relay.max_direct_attempts = parse_u32_arg(key, value)?;
        }
        ["network", "relay", "transport"] => {
            cfg.network.relay.transport = value
                .parse::<TransportPreference>()
                .with_context(|| format!("Invalid value '{value}' for network.relay.transport"))?;
        }
        ["tui", "show_timestamps"] => {
            cfg.tui.show_timestamps = parse_bool_arg(value)?;
        }
        ["tui", "color_scheme"] => {
            cfg.tui.color_scheme = value.trim().to_string();
        }
        ["tui", "auto_scroll"] => {
            cfg.tui.auto_scroll = parse_bool_arg(value)?;
        }
        ["tui", "confirm_commands"] => {
            cfg.tui.confirm_commands = parse_bool_arg(value)?;
        }
        ["cli", "output_format"] => {
            let normalized = value.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "tsv" | "json" => cfg.cli.output_format = normalized,
                _ => bail!("Unsupported output format '{}'. Use 'tsv' or 'json'", value),
            }
        }
        ["cli", "show_headers"] => {
            cfg.cli.show_headers = parse_bool_arg(value)?;
        }
        ["cli", "color_output"] => {
            let normalized = value.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "auto" | "always" | "never" => cfg.cli.color_output = normalized,
                _ => bail!(
                    "Invalid value for cli.color_output '{}'. Use auto|always|never",
                    value
                ),
            }
        }
        ["device", "name"] => set_device_string(cfg, true, value),
        ["device", "model"] => set_device_string(cfg, false, value),
        _ => {
            bail!("Unknown configuration key '{key}'")
        }
    }

    Ok(())
}

fn parse_bool_arg(value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => bail!(
            "Invalid boolean value '{}'. Use true/false, yes/no, on/off",
            value
        ),
    }
}

fn parse_u64_arg(key: &str, value: &str) -> Result<u64> {
    value
        .trim()
        .parse::<u64>()
        .with_context(|| format!("{} must be a positive integer", key))
}

fn parse_u32_arg(key: &str, value: &str) -> Result<u32> {
    value
        .trim()
        .parse::<u32>()
        .with_context(|| format!("{} must be a non-negative integer", key))
}

fn set_device_string(cfg: &mut ClientConfig, is_name: bool, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        if let Some(device) = cfg.device.as_mut() {
            if is_name {
                device.name = None;
            } else {
                device.model = None;
            }
        }
    } else {
        let device = cfg.device.get_or_insert(DeviceConfig {
            name: None,
            model: None,
        });
        if is_name {
            device.name = Some(trimmed.to_string());
        } else {
            device.model = Some(trimmed.to_string());
        }
    }

    if cfg
        .device
        .as_ref()
        .map(|d| d.name.is_none() && d.model.is_none())
        .unwrap_or(false)
    {
        cfg.device = None;
    }
}

fn to_discover_row(server: DiscoveredServer, registry: &ServerRegistry) -> DiscoverRow {
    let status = server
        .server_id
        .and_then(|id| registry.find_by_id(&id))
        .map(|_| "enrolled")
        .unwrap_or("available")
        .to_string();

    DiscoverRow {
        instance_name: server.instance_name,
        hostname: server.hostname,
        addresses: server.addresses,
        port: server.port,
        server_id: server.server_id.map(|id| id.to_string()),
        status,
        cert_fingerprint: server.cert_fingerprint,
        txt_properties: server.txt_properties,
        fullname: server.fullname,
    }
}

fn output_tsv_discovery(rows: &[DiscoverRow], cfg: &ClientConfig) -> Result<()> {
    let mut stdout = io::BufWriter::new(io::stdout().lock());

    if cfg.cli.show_headers {
        writeln!(stdout, "INSTANCE\tHOSTNAME\tIP\tPORT\tSERVER_ID\tSTATUS")?;
    }

    for row in rows {
        let ip = row
            .addresses
            .first()
            .cloned()
            .unwrap_or_else(|| "-".to_string());
        writeln!(
            stdout,
            "{}\t{}\t{}\t{}\t{}\t{}",
            row.instance_name,
            row.hostname,
            ip,
            row.port,
            row.server_id.as_deref().unwrap_or("-"),
            row.status
        )?;
    }

    stdout.flush()?;
    Ok(())
}

fn read_payload_from_stdin() -> Result<String> {
    println!("Paste enrollment payload JSON, then press Ctrl-D (Unix) or Ctrl-Z (Windows):");
    let mut buffer = String::new();
    io::stdin()
        .read_to_string(&mut buffer)
        .context("Failed to read payload from stdin")?;
    Ok(buffer)
}

fn parse_host_port(value: &str) -> Option<(String, u16)> {
    if let Ok(addr) = value.parse::<SocketAddr>() {
        return Some((addr.ip().to_string(), addr.port()));
    }
    if let Some(idx) = value.rfind(':') {
        let (host, port_str) = value.split_at(idx);
        if let Ok(port) = port_str[1..].parse::<u16>() {
            return Some((host.to_string(), port));
        }
    }
    None
}

fn output_tsv_registry(rows: &[RegistryRow], cfg: &ClientConfig) -> Result<()> {
    let mut stdout = io::BufWriter::new(io::stdout().lock());

    if cfg.cli.show_headers {
        writeln!(
            stdout,
            "SERVER_ID\tHOSTNAME\tADDRESSES\tMODE\tRELAY_URL\tLAST_SEEN"
        )?;
    }

    for row in rows {
        writeln!(
            stdout,
            "{}\t{}\t{}\t{}\t{}\t{}",
            row.server_id,
            row.hostname.as_deref().unwrap_or("-"),
            if row.addresses.is_empty() {
                "-".to_string()
            } else {
                row.addresses.join(", ")
            },
            row.relay_mode.as_deref().unwrap_or("direct-only"),
            row.relay_url
                .as_deref()
                .filter(|value| !value.is_empty())
                .unwrap_or("-"),
            row.last_seen.as_deref().unwrap_or("-")
        )?;
    }

    stdout.flush()?;
    Ok(())
}

fn output_json<T: Serialize>(value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    println!("{json}");
    Ok(())
}

#[derive(Debug, Serialize)]
struct DiscoverRow {
    instance_name: String,
    hostname: String,
    addresses: Vec<String>,
    port: u16,
    server_id: Option<String>,
    status: String,
    cert_fingerprint: Option<String>,
    txt_properties: std::collections::HashMap<String, String>,
    fullname: String,
}

#[derive(Debug, Serialize)]
struct RegistryRow {
    server_id: String,
    hostname: Option<String>,
    addresses: Vec<String>,
    relay_mode: Option<String>,
    relay_url: Option<String>,
    last_seen: Option<String>,
}

impl From<&ServerRegistryEntry> for RegistryRow {
    fn from(entry: &ServerRegistryEntry) -> Self {
        let (relay_mode, relay_url) = entry.relay.as_ref().map_or((None, None), |relay| {
            let mode = if relay.relay_required {
                "relay-only"
            } else {
                "direct+relay"
            };

            let url = if relay.relay_url.trim().is_empty() {
                None
            } else {
                Some(relay.relay_url.clone())
            };

            (Some(mode.to_string()), url)
        });

        Self {
            server_id: entry.id.to_string(),
            hostname: entry.hostname.clone(),
            addresses: collect_display_addresses(entry),
            relay_mode,
            relay_url,
            last_seen: entry.last_seen.clone(),
        }
    }
}

fn collect_display_addresses(entry: &ServerRegistryEntry) -> Vec<String> {
    fn push_unique(target: &mut Vec<String>, candidate: &str) {
        if candidate.trim().is_empty() {
            return;
        }
        if !target
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(candidate))
        {
            target.push(candidate.to_string());
        }
    }

    let mut addresses = Vec::new();
    for addr in &entry.addresses {
        push_unique(&mut addresses, addr);
    }
    if let Some(ip) = &entry.ip {
        push_unique(&mut addresses, ip);
    }
    if let Some(host) = &entry.hostname {
        push_unique(&mut addresses, host);
    }
    addresses
}
