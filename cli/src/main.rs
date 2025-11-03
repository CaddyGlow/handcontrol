use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use handcontrol_client_lib::{
    config::{self, ClientConfig},
    discover_servers, enroll_via_approval, enroll_via_qr,
    storage::{ServerRegistry, ServerRegistryEntry},
    ApprovalEnrollmentInput, DiscoveredServer, QrEnrollmentInput,
};
use serde::Serialize;
use std::fs;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tracing::debug;
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "handcontrol-cli",
    author,
    version,
    about = "HandControl command-line client",
    long_about = "HandControl CLI provides terminal-based access to HandControl servers."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Discover servers announced via mDNS
    Discover(DiscoverCommand),
    /// List enrolled servers
    ListServers(ListServersCommand),
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
    if tracing_subscriber::fmt::try_init().is_err() {
        debug!("Tracing subscriber already initialized");
    }

    let cli = Cli::parse();

    match cli.command {
        Command::Discover(cmd) => run_discover(cmd),
        Command::ListServers(cmd) => run_list_servers(cmd),
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
    }
    Ok(())
}

async fn run_enroll(command: EnrollCommand) -> Result<()> {
    match command {
        EnrollCommand::Qr(args) => run_enroll_qr(args).await,
        EnrollCommand::Approve(args) => run_enroll_approve(args).await,
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
    println!("Server address: {}", outcome.address);
    println!("Credentials stored in {}", outcome.cert_directory.display());

    Ok(())
}

async fn run_enroll_approve(args: ApproveEnrollCommand) -> Result<()> {
    let ApproveEnrollCommand {
        server,
        address,
        mut port,
        timeout,
        poll_interval,
        device_name,
        device_model,
    } = args;

    let cfg = config::load()?;
    let mut addresses: Vec<String> = Vec::new();
    let mut server_id_hint: Option<Uuid> = None;

    if let Some(addr) = address {
        if let Some((host, parsed_port)) = parse_host_port(&addr) {
            addresses.push(host);
            if port.is_none() {
                port = Some(parsed_port);
            }
        } else {
            addresses.push(addr);
        }
    } else {
        let discovered = discover_servers(&cfg.discovery)?;
        let query = server.to_lowercase();
        for entry in discovered {
            let mut matched = false;
            if let Some(id) = entry.server_id {
                if id.to_string().eq_ignore_ascii_case(&query) || id.to_string() == server {
                    matched = true;
                    server_id_hint = Some(id);
                }
            }
            if !matched
                && (entry.instance_name.eq_ignore_ascii_case(&server)
                    || entry.hostname.eq_ignore_ascii_case(&server))
            {
                matched = true;
                if server_id_hint.is_none() {
                    server_id_hint = entry.server_id;
                }
            }
            if matched {
                addresses.extend(entry.addresses.clone());
                if port.is_none() {
                    port = Some(entry.port);
                }
            }
        }

        if addresses.is_empty() {
            bail!(
                "Unable to resolve server '{server}'. Run 'handcontrol-cli discover' or supply --address"
            );
        }
    }

    addresses.sort();
    addresses.dedup();

    let resolved_device_name =
        device_name.or_else(|| cfg.device.as_ref().and_then(|d| d.name.clone()));
    let resolved_device_model =
        device_model.or_else(|| cfg.device.as_ref().and_then(|d| d.model.clone()));

    let input = ApprovalEnrollmentInput {
        addresses,
        port,
        server_id_hint,
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
        writeln!(stdout, "SERVER_ID\tHOSTNAME\tIP\tLAST_SEEN")?;
    }

    for row in rows {
        writeln!(
            stdout,
            "{}\t{}\t{}\t{}",
            row.server_id,
            row.hostname.as_deref().unwrap_or("-"),
            row.ip.as_deref().unwrap_or("-"),
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
    ip: Option<String>,
    last_seen: Option<String>,
}

impl From<&ServerRegistryEntry> for RegistryRow {
    fn from(entry: &ServerRegistryEntry) -> Self {
        Self {
            server_id: entry.id.to_string(),
            hostname: entry.hostname.clone(),
            ip: entry.ip.clone(),
            last_seen: entry.last_seen.clone(),
        }
    }
}
