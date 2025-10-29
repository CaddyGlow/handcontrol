use super::{NotificationProvider};
use anyhow::Result;
use tracing::info;

/// Fallback notification provider that logs to terminal
///
/// This provider is used when no platform-specific notification system is available.
/// It simply logs the notification details, which the user can see if they have
/// terminal access to the server.
pub struct FallbackNotificationProvider;

impl FallbackNotificationProvider {
    pub fn new() -> Self {
        Self
    }
}

impl NotificationProvider for FallbackNotificationProvider {
    fn show_pairing_notification(
        &self,
        device_name: &str,
        verification_code: &str,
    ) -> Result<bool> {
        // Log to console - user must have terminal access
        info!("=================================================================");
        info!("PAIRING REQUEST from device: {}", device_name);
        info!("Verification code displayed in notification (check terminal or notification center)");
        info!("=================================================================");
        info!("To approve or reject this request, use:");
        info!("  handcontrol approve <request-id>  # Accept the pairing");
        info!("  handcontrol reject <request-id>   # Reject the pairing");
        info!("=================================================================");

        Ok(true)
    }

    fn is_available(&self) -> bool {
        // Fallback is always available
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_provider_always_available() {
        let provider = FallbackNotificationProvider::new();
        assert!(provider.is_available());
    }

    #[test]
    fn test_fallback_show_notification() {
        let provider = FallbackNotificationProvider::new();
        let result = provider.show_pairing_notification("Test Device", "123-456");
        assert!(result.is_ok());
        assert!(result.unwrap());
    }
}
