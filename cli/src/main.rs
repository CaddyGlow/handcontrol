use anyhow::Result;
use clap::{Parser, Subcommand};
use handcontrol_client_lib::{
    config::{self, ClientConfig},
    discover_servers,
    storage::{ServerRegistry, ServerRegistryEntry},
    DiscoveredServer,
};
use serde::Serialize;
use std::io::{self, Write};
use tracing::debug;

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

fn main() -> Result<()> {
    if tracing_subscriber::fmt::try_init().is_err() {
        debug!("Tracing subscriber already initialized");
    }

    let cli = Cli::parse();

    match cli.command {
        Command::Discover(cmd) => run_discover(cmd),
        Command::ListServers(cmd) => run_list_servers(cmd),
        Command::Config { command } => run_config(command),
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
