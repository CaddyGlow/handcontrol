use anyhow::{Context, Result};
use std::net::SocketAddr;
use tracing::info;
use uuid::Uuid;

use crate::security::certificates::ServerCertificate;
use crate::security::enrollment::EnrollmentTokenManager;
use crate::utils::network::get_all_local_ips;
use crate::utils::qr::EnrollmentQrPayload;

/// Handle QR code enrollment
pub async fn handle_qr_enrollment(
    server_cert: &ServerCertificate,
    server_id: Uuid,
    bind_addr: SocketAddr,
    enrollment_manager: &EnrollmentTokenManager,
) -> Result<()> {
    info!("Generating QR code for enrollment...");

    // Generate enrollment token
    let token = enrollment_manager
        .generate_token()
        .context("Failed to generate enrollment token")?;

    info!("Generated enrollment token (expires in 5 minutes)");

    // Get server IPs from bind address
    let ips = if bind_addr.ip().is_unspecified() {
        // If binding to an unspecified address (0.0.0.0 or ::), get all usable local IPs
        let local_ips = get_all_local_ips();
        if local_ips.is_empty() {
            vec![bind_addr.ip().to_string()]
        } else {
            local_ips
        }
    } else {
        vec![bind_addr.ip().to_string()]
    };

    // Create QR payload
    let payload = EnrollmentQrPayload::new(
        ips,
        bind_addr.port(),
        server_cert.fingerprint_display(),
        token.token,
        server_id,
        None,
    );

    // Display QR code
    payload.display_qr()?;

    info!("Waiting for enrollment... (Press Ctrl+C to cancel)");

    // Wait for enrollment or timeout
    tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;

    info!("Enrollment session expired");

    Ok(())
}
