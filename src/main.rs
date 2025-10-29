use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use handcontrol::cli::enroll::handle_qr_enrollment;
use handcontrol::config::{default_config_path, load_config, validate_config};
use handcontrol::grpc::server::{start_server, RemoteControlService};
use handcontrol::security::certificates::ensure_server_certificate;
use handcontrol::security::enrollment::EnrollmentTokenManager;
use handcontrol::storage::clients::ClientStore;
use handcontrol::storage::paths;
use handcontrol::utils::logging;
use std::fs;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tracing::info;
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "handcontrol")]
#[command(about = "HandControl server - secure remote command execution", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate QR code for device enrollment
    Enroll {
        /// Generate QR code for enrollment
        #[arg(long)]
        qr: bool,
    },
    /// Start the HandControl server
    Serve,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Initialize logging first so we can log everything else
    logging::init_logging().context("Failed to initialize logging")?;

    // Handle commands
    match cli.command {
        Some(Commands::Enroll { qr }) => {
            if qr {
                handle_enroll_command().await?;
            } else {
                eprintln!("Please specify --qr for QR code enrollment");
                std::process::exit(1);
            }
        }
        Some(Commands::Serve) | None => {
            // Default: start server
            start_handcontrol_server().await?;
        }
    }

    Ok(())
}

async fn handle_enroll_command() -> Result<()> {
    info!("Starting enrollment command...");

    // Load configuration (needed for cert paths and settings)
    let config_path = default_config_path()
        .context("Failed to determine config path")?;

    if !config_path.exists() {
        anyhow::bail!(
            "Configuration file not found at {}. Please run the server first to generate default config.",
            config_path.display()
        );
    }

    let config = load_config(&config_path)
        .context("Failed to load configuration")?;

    // Check if QR enrollment is enabled
    if !config.security.enrollment.qr_code_enabled {
        anyhow::bail!("QR code enrollment is disabled in configuration");
    }

    // Load server certificate
    let cert_path = config.security.cert_path
        .as_ref()
        .map(|p| std::path::PathBuf::from(p))
        .unwrap_or_else(|| paths::server_cert_path().unwrap());
    let key_path = config.security.key_path
        .as_ref()
        .map(|p| std::path::PathBuf::from(p))
        .unwrap_or_else(|| paths::server_key_path().unwrap());

    let server_cert = ensure_server_certificate(&cert_path, &key_path)
        .context("Failed to load server certificate")?;

    // Generate server ID
    let server_id = Uuid::new_v4();

    // Create enrollment token manager
    let enrollment_manager = EnrollmentTokenManager::new(config.security.enrollment_token_ttl);

    // Parse bind address
    let addr: SocketAddr = format!("{}:{}", config.server.bind_address, config.server.port)
        .parse()
        .context("Failed to parse bind address")?;

    // Handle QR enrollment
    handle_qr_enrollment(&server_cert, server_id, addr, &enrollment_manager).await?;

    Ok(())
}

async fn start_handcontrol_server() -> Result<()> {
    info!("HandControl server starting...");

    // Determine config file path
    let config_path = default_config_path()
        .context("Failed to determine config path")?;

    info!("Config path: {}", config_path.display());

    // Check if config exists, if not create default
    if !config_path.exists() {
        info!("Config file not found, generating default configuration");

        let default_config = handcontrol::config::defaults::generate_default_config_toml()
            .context("Failed to generate default config")?;

        fs::write(&config_path, default_config)
            .with_context(|| format!("Failed to write default config to {}", config_path.display()))?;

        info!("Default configuration written to {}", config_path.display());
    }

    // Load configuration
    info!("Loading configuration...");
    let config = load_config(&config_path)
        .context("Failed to load configuration")?;

    // Validate configuration
    info!("Validating configuration...");
    validate_config(&config)
        .context("Configuration validation failed")?;

    info!(
        "Configuration loaded successfully: {} commands defined",
        config.command.len()
    );

    // Display server configuration
    info!(
        "Server will bind to {}:{}",
        config.server.bind_address, config.server.port
    );
    info!(
        "mDNS service name: {}",
        config.server.mdns_service_name
    );
    info!(
        "Enrollment modes: QR={}, Approval={}",
        config.security.enrollment.qr_code_enabled,
        config.security.enrollment.approval_enabled
    );

    // Initialize server certificate (Phase 2)
    info!("Initializing server certificate...");
    let cert_path = config.security.cert_path
        .as_ref()
        .map(|p| std::path::PathBuf::from(p))
        .unwrap_or_else(|| paths::server_cert_path().unwrap());
    let key_path = config.security.key_path
        .as_ref()
        .map(|p| std::path::PathBuf::from(p))
        .unwrap_or_else(|| paths::server_key_path().unwrap());

    let server_cert = ensure_server_certificate(&cert_path, &key_path)
        .context("Failed to initialize server certificate")?;

    info!(
        "Server certificate ready: {}",
        server_cert.fingerprint_display()
    );

    // Initialize client store for authorized clients
    info!("Initializing client store...");
    let clients_dir = paths::authorized_clients_dir()
        .context("Failed to determine authorized clients directory")?;
    let client_store = Arc::new(Mutex::new(
        ClientStore::new(clients_dir).context("Failed to initialize client store")?,
    ));

    // Generate server ID (UUID v4)
    let server_id = Uuid::new_v4();
    info!("Server ID: {}", server_id);

    // Initialize enrollment token manager
    let enrollment_ttl = config.security.enrollment_token_ttl;
    let enrollment_manager = EnrollmentTokenManager::new(enrollment_ttl);
    info!("Enrollment token TTL: {} seconds", enrollment_ttl);

    // Create gRPC service
    info!("Creating gRPC service...");
    let service = RemoteControlService::new(
        Arc::new(config),
        Arc::new(server_cert),
        server_id,
        client_store,
        enrollment_manager,
    );

    // Parse bind address
    let addr: SocketAddr = format!("{}:{}", "0.0.0.0", 50051)
        .parse()
        .context("Failed to parse bind address")?;

    info!("Server initialization complete");
    info!("gRPC server will listen on {}", addr);

    // TODO: Initialize mDNS service (Phase 7)

    // Start gRPC server
    start_server(addr, service)
        .await
        .context("gRPC server failed")?;

    Ok(())
}

