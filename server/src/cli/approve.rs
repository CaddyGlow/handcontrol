use anyhow::Result;
use tracing::info;

use crate::security::certificates::ClientCertificate;
use crate::security::pairing::PairingRequestManager;
use crate::storage::clients::ClientStore;

/// Handle manual approval of pending pairing requests
pub async fn handle_manual_approval(
    pairing_manager: &PairingRequestManager,
    _client_store: &mut ClientStore,
) -> Result<()> {
    info!("Checking for pending pairing requests...");

    let pending_count = pairing_manager.pending_count();

    if pending_count == 0 {
        println!("No pending pairing requests.");
        return Ok(());
    }

    println!("Found {} pending pairing request(s)", pending_count);
    println!("\nThis is a simplified approval interface for testing.");
    println!("In production, this would be handled via OS notifications.\n");

    // For now, just show a message
    // In a full implementation, we would list all pending requests and allow approval
    println!("To approve a pairing request, use the API or wait for notification integration.");

    Ok(())
}

/// Approve a specific pairing request (for testing)
pub fn approve_pairing_request(
    request_id: &str,
    pairing_manager: &PairingRequestManager,
    client_store: &mut ClientStore,
) -> Result<String> {
    // Get the pairing request
    let request = pairing_manager
        .get_request(request_id)
        .ok_or_else(|| anyhow::anyhow!("Pairing request not found"))?;

    if !request.is_pending() {
        anyhow::bail!("Pairing request is not pending");
    }

    // Parse client certificate
    let client_cert = ClientCertificate::from_der(request.client_certificate.clone());

    // Store client certificate with IP from pairing request
    let client_id = client_store.add_client(
        &client_cert,
        request.device_name.clone(),
        request.ip_address,
    )?;

    // Mark as approved
    pairing_manager.approve_request(request_id, client_id.clone())?;

    info!(
        "Approved pairing request {} for device {}, client_id={}",
        request_id, request.device_name, client_id
    );

    Ok(client_id)
}

/// Reject a specific pairing request (for testing)
pub fn reject_pairing_request(
    request_id: &str,
    pairing_manager: &PairingRequestManager,
) -> Result<()> {
    pairing_manager.reject_request(request_id)?;

    info!("Rejected pairing request {}", request_id);

    Ok(())
}
