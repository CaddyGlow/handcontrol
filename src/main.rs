use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use handcontrol::cli::enroll::handle_qr_enrollment;
use handcontrol::config::{default_config_path, load_config, validate_config};
use handcontrol::grpc::server::{start_server, RemoteControlService};
use handcontrol::mdns::service::MdnsService;
use handcontrol::notifications::NotificationManager;
use handcontrol::security::certificates::ensure_server_certificate;
use handcontrol::security::enrollment::EnrollmentTokenManager;
use handcontrol::security::pairing::PairingRequestManager;
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
    /// Approve a pending pairing request
    Approve {
        /// Request ID to approve
        request_id: String,
    },
    /// Reject a pending pairing request
    Reject {
        /// Request ID to reject
        request_id: String,
    },
    /// List pending pairing requests
    ListPending,
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
        Some(Commands::Approve { request_id }) => {
            handle_approve_command(&request_id).await?;
        }
        Some(Commands::Reject { request_id }) => {
            handle_reject_command(&request_id).await?;
        }
        Some(Commands::ListPending) => {
            handle_list_pending_command().await?;
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

async fn handle_approve_command(request_id: &str) -> Result<()> {
    info!("Approving pairing request: {}", request_id);

    // Note: In a production system, the pairing manager would persist its state
    // For now, we load it from the running server's state
    println!("Error: Cannot approve pairing request from CLI.");
    println!("The pairing manager state is only available while the server is running.");
    println!("Please use OS notifications to approve pairing requests.");
    println!("\nAlternatively, you can:");
    println!("  1. Check the server logs for the verification code");
    println!("  2. Ensure the verification codes match on both devices");
    println!("  3. Wait for the approval timeout to expire and try again");

    anyhow::bail!("Pairing approval requires server to be running")
}

async fn handle_reject_command(request_id: &str) -> Result<()> {
    info!("Rejecting pairing request: {}", request_id);

    println!("Error: Cannot reject pairing request from CLI.");
    println!("The pairing manager state is only available while the server is running.");
    println!("Please use OS notifications to reject pairing requests.");

    anyhow::bail!("Pairing rejection requires server to be running")
}

async fn handle_list_pending_command() -> Result<()> {
    info!("Listing pending pairing requests");

    println!("Error: Cannot list pairing requests from CLI.");
    println!("The pairing manager state is only available while the server is running.");
    println!("Please check the server logs for pending pairing requests.");

    anyhow::bail!("Listing pairing requests requires server to be running")
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

    // Initialize pairing request manager
    let pairing_timeout = config.security.enrollment.approval_timeout_seconds;
    let pairing_manager = PairingRequestManager::new(pairing_timeout);
    info!("Pairing request timeout: {} seconds", pairing_timeout);

    // Initialize notification manager
    info!("Initializing notification manager...");
    let notification_manager = NotificationManager::new();
    if notification_manager.is_available() {
        info!("Notification system available");
    } else {
        info!("Notification system unavailable, using fallback");
    }

    // Get values needed for mDNS before moving into Arc
    let mdns_instance_name = config.server.mdns_instance_name.clone();
    let mdns_port = config.server.port;
    let cert_fingerprint = server_cert.fingerprint_display();

    // Create gRPC service
    info!("Creating gRPC service...");
    let service = RemoteControlService::new(
        Arc::new(config),
        Arc::new(server_cert),
        server_id,
        client_store,
        enrollment_manager,
        pairing_manager,
        notification_manager,
    );

    // Parse bind address
    let addr: SocketAddr = format!("{}:{}", "0.0.0.0", 50051)
        .parse()
        .context("Failed to parse bind address")?;

    info!("Server initialization complete");
    info!("gRPC server will listen on {}", addr);

    // Initialize mDNS service (Phase 7)
    info!("Initializing mDNS service...");
    let instance_name = mdns_instance_name
        .unwrap_or_else(|| {
            hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "HandControl".to_string())
        });

    let mdns_service = MdnsService::new(
        instance_name.clone(),
        mdns_port,
        server_id,
        cert_fingerprint,
    );

    if let Err(e) = mdns_service.start() {
        tracing::warn!("Failed to start mDNS service (continuing without discovery): {}", e);
    } else {
        info!("mDNS service started: instance_name={}", instance_name);
    }

    // Start gRPC server with TLS
    info!("Starting gRPC server with TLS...");
    start_server(addr, service, cert_path, key_path)
        .await
        .context("gRPC server failed")?;

    // Clean shutdown: stop mDNS service
    if let Err(e) = mdns_service.stop() {
        tracing::warn!("Failed to stop mDNS service during shutdown: {}", e);
    }

    Ok(())
}

