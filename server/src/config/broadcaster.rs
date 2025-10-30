use tokio::sync::broadcast;

/// Notification sent when config is reloaded
#[derive(Clone, Debug)]
pub struct ConfigUpdateNotification {
    pub version: u64,
    pub timestamp_ms: i64,
}

/// Broadcasts config update notifications to multiple subscribers
pub struct ConfigBroadcaster {
    tx: broadcast::Sender<ConfigUpdateNotification>,
}

impl ConfigBroadcaster {
    /// Create a new broadcaster with the specified channel capacity
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Broadcast a config update notification to all subscribers
    /// Ignores send errors (no active subscribers is fine)
    pub fn broadcast(&self, notification: ConfigUpdateNotification) {
        let _ = self.tx.send(notification);
    }

    /// Subscribe to config update notifications
    pub fn subscribe(&self) -> broadcast::Receiver<ConfigUpdateNotification> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[tokio::test]
    async fn test_broadcaster_single_subscriber() {
        let broadcaster = ConfigBroadcaster::new(10);
        let mut rx = broadcaster.subscribe();

        let notification = ConfigUpdateNotification {
            version: 1,
            timestamp_ms: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
        };

        broadcaster.broadcast(notification.clone());

        let received = rx.recv().await.unwrap();
        assert_eq!(received.version, 1);
    }

    #[tokio::test]
    async fn test_broadcaster_multiple_subscribers() {
        let broadcaster = ConfigBroadcaster::new(10);
        let mut rx1 = broadcaster.subscribe();
        let mut rx2 = broadcaster.subscribe();

        let notification = ConfigUpdateNotification {
            version: 2,
            timestamp_ms: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
        };

        broadcaster.broadcast(notification.clone());

        let received1 = rx1.recv().await.unwrap();
        let received2 = rx2.recv().await.unwrap();

        assert_eq!(received1.version, 2);
        assert_eq!(received2.version, 2);
    }

    #[tokio::test]
    async fn test_broadcaster_no_subscribers() {
        let broadcaster = ConfigBroadcaster::new(10);

        let notification = ConfigUpdateNotification {
            version: 3,
            timestamp_ms: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
        };

        // Should not panic even with no subscribers
        broadcaster.broadcast(notification);
    }
}
