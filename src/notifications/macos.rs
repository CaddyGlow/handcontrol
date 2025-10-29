use super::NotificationProvider;
use anyhow::Result;
use tracing::debug;

/// macOS notification provider (placeholder)
///
/// This will be implemented using macOS Notification Center in a future update.
/// For now, it returns false for is_available() so the fallback provider is used.
pub struct MacosNotificationProvider;

impl MacosNotificationProvider {
    pub fn new() -> Self {
        Self
    }
}

impl NotificationProvider for MacosNotificationProvider {
    fn show_pairing_notification(
        &self,
        _device_name: &str,
        _verification_code: &str,
    ) -> Result<bool> {
        debug!("macOS notifications not yet implemented");
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
    fn test_macos_provider_not_available() {
        let provider = MacosNotificationProvider::new();
        assert!(!provider.is_available());
    }
}
