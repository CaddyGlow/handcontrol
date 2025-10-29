use anyhow::{Context, Result};
use handcontrol::config::{default_config_path, load_config, validate_config};
use handcontrol::security::certificates::ensure_server_certificate;
use handcontrol::storage::paths;
use handcontrol::utils::logging;
use std::fs;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging first so we can log everything else
    logging::init_logging().context("Failed to initialize logging")?;

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

    // TODO: Initialize gRPC server (Phase 3)
    // TODO: Initialize mDNS service (Phase 7)
    info!("Server initialization complete");
    info!("Phase 2 security components ready - awaiting Phase 3 (gRPC)");

    // Keep the server running for now
    tokio::signal::ctrl_c()
        .await
        .context("Failed to listen for ctrl-c signal")?;

    info!("Shutdown signal received, stopping server...");

    Ok(())
}

