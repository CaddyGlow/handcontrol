use anyhow::Result;

pub mod fallback;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

/// Action that a user can take in response to a notification
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationAction {
    Accept,
    Reject,
}

/// Response from a notification
#[derive(Debug, Clone)]
pub struct NotificationResponse {
    pub action: NotificationAction,
}

/// Trait for platform-specific notification implementations
pub trait NotificationProvider: Send + Sync {
    /// Show a pairing notification with Accept/Reject actions
    /// Returns true if notification was shown successfully
    fn show_pairing_notification(&self, device_name: &str, verification_code: &str)
    -> Result<bool>;

    /// Check if notifications are available on this platform
    fn is_available(&self) -> bool;
}

/// Manager that handles notifications across platforms
pub struct NotificationManager {
    provider: Box<dyn NotificationProvider>,
}

impl NotificationManager {
    /// Create a new notification manager with the appropriate provider for this platform
    pub fn new() -> Self {
        let provider: Box<dyn NotificationProvider> = Self::create_provider();
        Self { provider }
    }

    /// Create the appropriate notification provider for the current platform
    fn create_provider() -> Box<dyn NotificationProvider> {
        #[cfg(target_os = "linux")]
        {
            // Try Linux D-Bus notifications first
            if linux::LinuxNotificationProvider::new().is_available() {
                return Box::new(linux::LinuxNotificationProvider::new());
            }
        }

        #[cfg(target_os = "windows")]
        {
            // Try Windows Toast notifications first
            if windows::WindowsNotificationProvider::new().is_available() {
                return Box::new(windows::WindowsNotificationProvider::new());
            }
        }

        #[cfg(target_os = "macos")]
        {
            // Try macOS Notification Center first
            if macos::MacosNotificationProvider::new().is_available() {
                return Box::new(macos::MacosNotificationProvider::new());
            }
        }

        // Fallback to terminal notifications
        Box::new(fallback::FallbackNotificationProvider::new())
    }

    /// Show a pairing notification
    pub fn show_pairing_notification(
        &self,
        device_name: &str,
        verification_code: &str,
    ) -> Result<bool> {
        self.provider
            .show_pairing_notification(device_name, verification_code)
    }

    /// Check if notifications are available
    pub fn is_available(&self) -> bool {
        self.provider.is_available()
    }
}

impl Default for NotificationManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_manager_creation() {
        let manager = NotificationManager::new();
        // Should always have at least the fallback provider
        assert!(manager.is_available());
    }
}
