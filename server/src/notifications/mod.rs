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
    fn show_pairing_notification(
        &self,
        device_name: &str,
        verification_code: &str,
        request_id: &str,
    ) -> Result<bool>;

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

    /// Create a notification manager with a custom provider (useful for testing)
    pub fn with_provider(provider: Box<dyn NotificationProvider>) -> Self {
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
        request_id: &str,
    ) -> Result<bool> {
        self.provider
            .show_pairing_notification(device_name, verification_code, request_id)
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
    use std::sync::{Arc, Mutex};

    /// Mock notification provider for testing that tracks calls without showing real notifications
    #[derive(Clone)]
    pub struct MockNotificationProvider {
        calls: Arc<Mutex<Vec<NotificationCall>>>,
        should_succeed: bool,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct NotificationCall {
        pub device_name: String,
        pub verification_code: String,
        pub request_id: String,
    }

    impl MockNotificationProvider {
        pub fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                should_succeed: true,
            }
        }

        /// Create a mock provider that will fail
        pub fn new_failing() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                should_succeed: false,
            }
        }

        /// Get all recorded notification calls
        pub fn get_calls(&self) -> Vec<NotificationCall> {
            self.calls.lock().unwrap().clone()
        }

        /// Check if any notifications were shown
        pub fn was_called(&self) -> bool {
            !self.calls.lock().unwrap().is_empty()
        }

        /// Get the number of notifications shown
        pub fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl NotificationProvider for MockNotificationProvider {
        fn show_pairing_notification(
            &self,
            device_name: &str,
            verification_code: &str,
            request_id: &str,
        ) -> Result<bool> {
            self.calls.lock().unwrap().push(NotificationCall {
                device_name: device_name.to_string(),
                verification_code: verification_code.to_string(),
                request_id: request_id.to_string(),
            });

            if self.should_succeed {
                Ok(true)
            } else {
                Err(anyhow::anyhow!("Mock notification failed"))
            }
        }

        fn is_available(&self) -> bool {
            true
        }
    }

    #[test]
    fn test_notification_manager_creation() {
        let manager = NotificationManager::new();
        // Should always have at least the fallback provider
        assert!(manager.is_available());
    }

    #[test]
    fn test_mock_notification_provider() {
        let mock = MockNotificationProvider::new();
        assert!(!mock.was_called());
        assert_eq!(mock.call_count(), 0);

        // Show a notification
        let result = mock.show_pairing_notification("TestDevice", "123456", "req-1");
        assert!(result.is_ok());
        assert!(result.unwrap());

        // Verify it was recorded
        assert!(mock.was_called());
        assert_eq!(mock.call_count(), 1);

        let calls = mock.get_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].device_name, "TestDevice");
        assert_eq!(calls[0].verification_code, "123456");
        assert_eq!(calls[0].request_id, "req-1");
    }

    #[test]
    fn test_mock_notification_multiple_calls() {
        let mock = MockNotificationProvider::new();

        // Show multiple notifications
        mock.show_pairing_notification("Device1", "111111", "req-1")
            .unwrap();
        mock.show_pairing_notification("Device2", "222222", "req-2")
            .unwrap();
        mock.show_pairing_notification("Device3", "333333", "req-3")
            .unwrap();

        assert_eq!(mock.call_count(), 3);

        let calls = mock.get_calls();
        assert_eq!(calls[0].device_name, "Device1");
        assert_eq!(calls[1].device_name, "Device2");
        assert_eq!(calls[2].device_name, "Device3");
    }

    #[test]
    fn test_mock_notification_failing_provider() {
        let mock = MockNotificationProvider::new_failing();

        let result = mock.show_pairing_notification("TestDevice", "123456", "req-1");
        assert!(result.is_err());

        // Call should still be recorded even on failure
        assert!(mock.was_called());
        assert_eq!(mock.call_count(), 1);
    }

    #[test]
    fn test_notification_manager_with_mock_provider() {
        let mock = MockNotificationProvider::new();
        let mock_clone = mock.clone();

        let manager = NotificationManager::with_provider(Box::new(mock));
        assert!(manager.is_available());

        // Use the manager to show a notification
        let result = manager.show_pairing_notification("MyPhone", "654321", "test-request");
        assert!(result.is_ok());
        assert!(result.unwrap());

        // Verify via the cloned mock that the call was made
        assert!(mock_clone.was_called());
        assert_eq!(mock_clone.call_count(), 1);

        let calls = mock_clone.get_calls();
        assert_eq!(calls[0].device_name, "MyPhone");
        assert_eq!(calls[0].verification_code, "654321");
        assert_eq!(calls[0].request_id, "test-request");
    }
}
