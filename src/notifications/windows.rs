use super::NotificationProvider;
use anyhow::Result;
use tracing::debug;

/// Windows notification provider (placeholder)
///
/// This will be implemented using Windows Toast notifications in a future update.
/// For now, it returns false for is_available() so the fallback provider is used.
pub struct WindowsNotificationProvider;

impl WindowsNotificationProvider {
    pub fn new() -> Self {
        Self
    }
}

impl NotificationProvider for WindowsNotificationProvider {
    fn show_pairing_notification(
        &self,
        _device_name: &str,
        _verification_code: &str,
    ) -> Result<bool> {
        debug!("Windows notifications not yet implemented");
        Ok(false)
    }

    fn is_available(&self) -> bool {
        // Not yet implemented
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_windows_provider_not_available() {
        let provider = WindowsNotificationProvider::new();
        assert!(!provider.is_available());
    }
}
