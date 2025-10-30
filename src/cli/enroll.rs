use anyhow::{Context, Result};
use std::net::SocketAddr;
use tracing::info;
use uuid::Uuid;

use crate::security::certificates::ServerCertificate;
use crate::security::enrollment::EnrollmentTokenManager;
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

    // Get server IP from bind address
    let ip = if bind_addr.ip().is_unspecified() {
        // If binding to an unspecified address (0.0.0.0 or ::), try to get a local IP
        get_local_ip().unwrap_or_else(|| bind_addr.ip().to_string())
    } else {
        bind_addr.ip().to_string()
    };

    // Create QR payload
    let payload = EnrollmentQrPayload::new(
        ip,
        bind_addr.port(),
        server_cert.fingerprint_display(),
        token.token,
        server_id,
    );

    // Display QR code
    payload.display_qr()?;

    info!("Waiting for enrollment... (Press Ctrl+C to cancel)");

    // Wait for enrollment or timeout
    tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;

    info!("Enrollment session expired");

    Ok(())
}

/// Get local IP address (best effort)
fn get_local_ip() -> Option<String> {
    use std::net::UdpSocket;

    // Try to connect to a public DNS server to determine local IP
    // This doesn't actually send any data
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let local_addr = socket.local_addr().ok()?;

    Some(local_addr.ip().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_local_ip() {
        // This may fail in some environments (e.g., no network)
        // So we just test it doesn't panic
        let _ip = get_local_ip();
    }
}
