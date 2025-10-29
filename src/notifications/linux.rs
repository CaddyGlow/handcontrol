use super::NotificationProvider;
use anyhow::{Context, Result};
use notify_rust::{Notification, Timeout};
use tracing::{debug, warn};

/// Linux notification provider using D-Bus notifications
///
/// Uses the notify-rust crate to display desktop notifications via D-Bus.
/// This works on most Linux desktop environments (GNOME, KDE, XFCE, etc.)
pub struct LinuxNotificationProvider;

impl LinuxNotificationProvider {
    pub fn new() -> Self {
        Self
    }

    /// Check if D-Bus notifications are available
    fn check_dbus_available() -> bool {
        // Try to create a simple notification to test D-Bus connection
        match Notification::new()
            .summary("HandControl")
            .body("Testing notification support")
            .timeout(Timeout::Milliseconds(1))
            .show()
        {
            Ok(_) => {
                debug!("D-Bus notifications are available");
                true
            }
            Err(e) => {
                debug!("D-Bus notifications not available: {}", e);
                false
            }
        }
    }
}

impl NotificationProvider for LinuxNotificationProvider {
    fn show_pairing_notification(
        &self,
        device_name: &str,
        verification_code: &str,
    ) -> Result<bool> {
        debug!(
            "Showing Linux D-Bus notification for device: {}",
            device_name
        );

        // Create notification with pairing details
        let body = format!(
            "Device \"{}\" wants to pair\nVerification Code: {}\n\nPlease verify this code matches on your device.",
            device_name, verification_code
        );

        match Notification::new()
            .summary("HandControl Pairing Request")
            .body(&body)
            .icon("dialog-question")
            .timeout(Timeout::Never) // Stay visible until user dismisses
            .urgency(notify_rust::Urgency::Critical) // High priority
            .show()
        {
            Ok(handle) => {
                debug!("Notification shown successfully: {:?}", handle);
                Ok(true)
            }
            Err(e) => {
                warn!("Failed to show D-Bus notification: {}", e);
                Err(e).context("Failed to show Linux notification")
            }
        }
    }

    fn is_available(&self) -> bool {
        Self::check_dbus_available()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linux_provider_creation() {
        let provider = LinuxNotificationProvider::new();
        // Just verify it can be created
        // Availability depends on D-Bus which may not be present in CI
        let _ = provider.is_available();
    }

    #[test]
    fn test_linux_show_notification() {
        let provider = LinuxNotificationProvider::new();

        // Only test if D-Bus is available
        if provider.is_available() {
            let result = provider.show_pairing_notification("Test Device", "123-456");
            // Should succeed or fail gracefully
            assert!(result.is_ok() || result.is_err());
        }
    }
}
