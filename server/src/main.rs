use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use handcontrol::config::{
    ConfigBroadcaster, ConfigWatcher, default_config_path, load_config, validate_config,
};
use handcontrol::grpc::proto::remote_control_client::RemoteControlClient;
use handcontrol::grpc::proto::{
    ApprovePairingRequest, GenerateEnrollmentQrRequest, ListPendingPairingsRequest,
};
use handcontrol::grpc::server::{RemoteControlService, start_server};
use handcontrol::mdns::service::MdnsService;
use handcontrol::notifications::NotificationManager;
use handcontrol::relay::{RelayClient, TokenIssuer};
use handcontrol::security::certificates::ensure_server_certificate;
use handcontrol::security::enrollment::EnrollmentTokenManager;
use handcontrol::security::pairing::PairingRequestManager;
use handcontrol::storage;
use handcontrol::storage::clients::ClientStore;
use handcontrol::storage::paths;
use handcontrol::utils::logging;
use handcontrol::utils::network::get_all_local_ips;
use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, RwLock};
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint};
use tracing::info;

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

    // Load configuration
    let config_path = default_config_path().context("Failed to determine config path")?;

    if !config_path.exists() {
        anyhow::bail!(
            "Configuration file not found at {}. Please run the server first to generate default config.",
            config_path.display()
        );
    }

    let config = load_config(&config_path).context("Failed to load configuration")?;

    // Check if QR enrollment is enabled
    if !config.security.enrollment.qr_code_enabled {
        anyhow::bail!("QR code enrollment is disabled in configuration");
    }

    // Connect to running server via gRPC
    let host = match config.server.bind_address.as_str() {
        "0.0.0.0" | "::" => "127.0.0.1",
        other => other,
    }
    .to_string();

    let cert_path = if let Some(path) = config.security.cert_path.as_ref() {
        std::path::PathBuf::from(path)
    } else {
        paths::server_cert_path().context("Failed to determine server certificate path")?
    };

    let server_cert = fs::read(&cert_path).with_context(|| {
        format!(
            "Failed to read server certificate from {}",
            cert_path.display()
        )
    })?;

    // Format host for URI (IPv6 addresses need square brackets)
    let uri_host = if let Ok(ip) = host.parse::<IpAddr>() {
        match ip {
            IpAddr::V6(_) => format!("[{}]", host),
            IpAddr::V4(_) => host.clone(),
        }
    } else {
        host.clone()
    };

    let target = format!("https://{}:{}", uri_host, config.server.port);
    let endpoint = Endpoint::from_shared(target.clone())
        .with_context(|| format!("Invalid server endpoint URL: {}", target))?;

    let domain_name = if host.parse::<IpAddr>().is_ok() {
        "localhost".to_string()
    } else {
        host.clone()
    };

    let tls_config = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(server_cert))
        .domain_name(domain_name);

    let channel = endpoint
        .tls_config(tls_config)
        .context("Failed to configure TLS for CLI client")?
        .connect()
        .await
        .context("Failed to connect to HandControl server. Is the server running?")?;

    let mut client = RemoteControlClient::new(channel);

    // Call GenerateEnrollmentQR RPC
    let response = client
        .generate_enrollment_qr(GenerateEnrollmentQrRequest {})
        .await
        .context("GenerateEnrollmentQR RPC failed")?
        .into_inner();

    if !response.success {
        anyhow::bail!(
            "Server failed to generate enrollment QR: {}",
            response.error_message
        );
    }

    // Parse QR payload and display it
    let payload: handcontrol::utils::qr::EnrollmentQrPayload =
        serde_json::from_str(&response.qr_payload)
            .context("Failed to parse QR payload from server")?;

    // Display QR code
    payload.display_qr()?;

    info!("Waiting for enrollment... (Press Ctrl+C to cancel)");

    // Wait for TTL or Ctrl+C
    tokio::time::sleep(tokio::time::Duration::from_secs(
        response.ttl_seconds as u64,
    ))
    .await;

    info!("Enrollment session expired");

    Ok(())
}

async fn handle_approve_command(request_id: &str) -> Result<()> {
    info!("Approving pairing request: {}", request_id);

    let config_path = default_config_path().context("Failed to determine config path")?;
    let config = load_config(&config_path).context("Failed to load configuration")?;

    let host = match config.server.bind_address.as_str() {
        "0.0.0.0" | "::" => {
            // Use primary IP from network enumeration
            let ips = get_all_local_ips();
            ips.first()
                .cloned()
                .unwrap_or_else(|| "127.0.0.1".to_string())
        }
        other => other.to_string(),
    };

    let cert_path = if let Some(path) = config.security.cert_path.as_ref() {
        std::path::PathBuf::from(path)
    } else {
        paths::server_cert_path().context("Failed to determine server certificate path")?
    };

    let server_cert = fs::read(&cert_path).with_context(|| {
        format!(
            "Failed to read server certificate from {}",
            cert_path.display()
        )
    })?;

    // Format host for URI (IPv6 addresses need square brackets)
    let uri_host = if let Ok(ip) = host.parse::<IpAddr>() {
        match ip {
            IpAddr::V6(_) => format!("[{}]", host),
            IpAddr::V4(_) => host.clone(),
        }
    } else {
        host.clone()
    };

    let target = format!("https://{}:{}", uri_host, config.server.port);
    let endpoint = Endpoint::from_shared(target.clone())
        .with_context(|| format!("Invalid server endpoint URL: {}", target))?;

    let domain_name = if host.parse::<IpAddr>().is_ok() {
        "localhost".to_string()
    } else {
        host.clone()
    };

    let tls_config = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(server_cert))
        .domain_name(domain_name);

    let channel = endpoint
        .tls_config(tls_config)
        .context("Failed to configure TLS for CLI client")?
        .connect()
        .await
        .context("Failed to connect to HandControl server")?;

    let mut client = RemoteControlClient::new(channel);

    let response = client
        .approve_pairing(ApprovePairingRequest {
            pairing_request_id: request_id.to_string(),
        })
        .await
        .context("ApprovePairing RPC failed")?
        .into_inner();

    if response.success {
        println!("Pairing request approved. client_id={}", response.client_id);
        Ok(())
    } else {
        anyhow::bail!(
            "Server rejected pairing approval: {}",
            response.error_message
        )
    }
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

    let config_path = default_config_path().context("Failed to determine config path")?;
    let config = load_config(&config_path).context("Failed to load configuration")?;

    let host = match config.server.bind_address.as_str() {
        "0.0.0.0" | "::" => {
            // Use primary IP from network enumeration
            let ips = get_all_local_ips();
            ips.first()
                .cloned()
                .unwrap_or_else(|| "127.0.0.1".to_string())
        }
        other => other.to_string(),
    };

    let cert_path = if let Some(path) = config.security.cert_path.as_ref() {
        std::path::PathBuf::from(path)
    } else {
        paths::server_cert_path().context("Failed to determine server certificate path")?
    };

    let server_cert = fs::read(&cert_path).with_context(|| {
        format!(
            "Failed to read server certificate from {}",
            cert_path.display()
        )
    })?;

    // Format host for URI (IPv6 addresses need square brackets)
    let uri_host = if let Ok(ip) = host.parse::<IpAddr>() {
        match ip {
            IpAddr::V6(_) => format!("[{}]", host),
            IpAddr::V4(_) => host.clone(),
        }
    } else {
        host.clone()
    };

    let target = format!("https://{}:{}", uri_host, config.server.port);
    let endpoint = Endpoint::from_shared(target.clone())
        .with_context(|| format!("Invalid server endpoint URL: {}", target))?;

    let domain_name = if host.parse::<IpAddr>().is_ok() {
        "localhost".to_string()
    } else {
        host.clone()
    };

    let tls_config = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(server_cert))
        .domain_name(domain_name);

    let channel = endpoint
        .tls_config(tls_config)
        .context("Failed to configure TLS for CLI client")?
        .connect()
        .await
        .context("Failed to connect to HandControl server")?;

    let mut client = RemoteControlClient::new(channel);

    let response = client
        .list_pending_pairings(ListPendingPairingsRequest {})
        .await
        .context("ListPendingPairings RPC failed")?
        .into_inner();

    if response.requests.is_empty() {
        eprintln!("No pending pairing requests.");
        return Ok(());
    }

    // Format output for easy parsing and fzf
    // Format: request_id | device_name | device_model | pin | expires_in_seconds | ip_address
    for req in response.requests {
        let model = if req.device_model.is_empty() {
            "Unknown".to_string()
        } else {
            req.device_model
        };

        let ip = if req.ip_address.is_empty() {
            "unknown".to_string()
        } else {
            req.ip_address
        };

        println!(
            "{}\t{}\t{}\t{}\t{}s\t{}",
            req.request_id,
            req.device_name,
            model,
            req.verification_code,
            req.seconds_remaining,
            ip
        );
    }

    Ok(())
}

async fn start_handcontrol_server() -> Result<()> {
    info!("HandControl server starting...");

    // Determine config file path
    let config_path = default_config_path().context("Failed to determine config path")?;

    info!("Config path: {}", config_path.display());

    // Check if config exists, if not create default
    if !config_path.exists() {
        info!("Config file not found, generating default configuration");

        let default_config = handcontrol::config::defaults::generate_default_config_toml()
            .context("Failed to generate default config")?;

        fs::write(&config_path, default_config).with_context(|| {
            format!(
                "Failed to write default config to {}",
                config_path.display()
            )
        })?;

        info!("Default configuration written to {}", config_path.display());
    }

    // Load configuration
    info!("Loading configuration...");
    let config = load_config(&config_path).context("Failed to load configuration")?;

    // Validate configuration
    info!("Validating configuration...");
    validate_config(&config).context("Configuration validation failed")?;

    info!(
        "Configuration loaded successfully: {} commands defined",
        config.command.len()
    );

    // Display server configuration
    info!(
        "Server will bind to {}:{}",
        config.server.bind_address, config.server.port
    );
    info!("mDNS service name: {}", config.server.mdns_service_name);
    info!(
        "Enrollment modes: QR={}, Approval={}",
        config.security.enrollment.qr_code_enabled, config.security.enrollment.approval_enabled
    );

    // Initialize server certificate (Phase 2)
    info!("Initializing server certificate...");
    let cert_path = config
        .security
        .cert_path
        .as_ref()
        .map(|p| std::path::PathBuf::from(p))
        .unwrap_or_else(|| paths::server_cert_path().unwrap());
    let key_path = config
        .security
        .key_path
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

    // Load or create persistent server ID
    let server_id = storage::server_identity::load_or_create_server_id()
        .context("Failed to initialize server ID")?;

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

    // Initialize relay infrastructure
    let token_issuer = if config.relay.enabled {
        info!("Relay support enabled, initializing token issuer...");
        let relay_key_path = paths::config_dir()
            .context("Failed to determine config directory")?
            .join("relay-key.pem");

        let issuer = Arc::new(
            TokenIssuer::new(server_id, &relay_key_path)
                .context("Failed to initialize relay token issuer")?,
        );

        info!("Relay token issuer initialized");
        Some(issuer)
    } else {
        None
    };

    // Get values needed for networking/mDNS before moving into Arc
    let mdns_instance_name = config.server.mdns_instance_name.clone();
    let server_port = config.server.port;
    let bind_address = config.server.bind_address.clone();
    let cert_fingerprint = server_cert.fingerprint_display();

    // Create config version tracking
    let config_version = Arc::new(AtomicU64::new(1));
    let config_broadcaster = Arc::new(ConfigBroadcaster::new(100));
    let last_config_update = Arc::new(AtomicU64::new(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
    ));

    // Wrap config in RwLock for hot-reload support
    let config_arc = Arc::new(RwLock::new(config));

    // Start config watcher task
    info!("Starting config file watcher...");
    let watcher = ConfigWatcher::new(
        config_path.clone(),
        config_arc.clone(),
        config_version.clone(),
        config_broadcaster.clone(),
        last_config_update.clone(),
    );
    tokio::spawn(async move {
        if let Err(e) = watcher.start().await {
            tracing::error!("Config watcher failed: {}", e);
        }
    });

    // Create gRPC service
    info!("Creating gRPC service...");
    let service = RemoteControlService::new(
        config_arc.clone(),
        Arc::new(server_cert),
        server_id,
        client_store,
        enrollment_manager,
        pairing_manager,
        notification_manager,
        config_version,
        config_broadcaster,
        last_config_update,
        token_issuer.clone(),
    );

    // Parse bind address
    let ip_addr: IpAddr = bind_address
        .parse()
        .with_context(|| format!("Failed to parse bind address '{}'", bind_address))?;
    let addr = SocketAddr::new(ip_addr, server_port);

    info!("Server initialization complete");
    info!("gRPC server will listen on {}", addr);

    // Initialize mDNS service (Phase 7)
    info!("Initializing mDNS service...");
    let instance_name = mdns_instance_name.unwrap_or_else(|| {
        hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "HandControl".to_string())
    });

    let mdns_service = MdnsService::new(
        instance_name.clone(),
        server_port,
        server_id,
        cert_fingerprint,
    );

    if let Err(e) = mdns_service.start() {
        tracing::warn!(
            "Failed to start mDNS service (continuing without discovery): {}",
            e
        );
    } else {
        info!("mDNS service started: instance_name={}", instance_name);
    }

    // Start relay client if enabled
    if let Some(ref issuer) = token_issuer {
        let relay_config = config_arc.read().unwrap().relay.clone();
        if relay_config.enabled {
            info!("Starting relay client...");
            let relay_client = Arc::new(RelayClient::new(
                relay_config,
                server_id,
                issuer.clone(),
                addr,
            ));

            // Spawn relay client task
            let relay_client_clone = relay_client.clone();
            tokio::spawn(async move {
                relay_client_clone.start().await;
            });

            info!("Relay client started");
        }
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
