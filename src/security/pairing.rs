use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// Status of a pairing request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingRequestStatus {
    Pending,
    Approved,
    Rejected,
    Timeout,
}

/// A pending pairing request
#[derive(Debug, Clone)]
pub struct PairingRequest {
    pub request_id: String,
    pub device_name: String,
    pub device_model: Option<String>,
    pub client_certificate: Vec<u8>,
    pub verification_code: String,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub status: PairingRequestStatus,
    pub client_id: Option<String>,
}

impl PairingRequest {
    /// Create a new pairing request
    pub fn new(
        device_name: String,
        device_model: Option<String>,
        client_certificate: Vec<u8>,
        verification_code: String,
        timeout_seconds: u64,
    ) -> Self {
        let now = OffsetDateTime::now_utc();
        let request_id = Uuid::new_v4().to_string();

        Self {
            request_id,
            device_name,
            device_model,
            client_certificate,
            verification_code,
            created_at: now,
            expires_at: now + Duration::seconds(timeout_seconds as i64),
            status: PairingRequestStatus::Pending,
            client_id: None,
        }
    }

    /// Check if request is expired
    pub fn is_expired(&self) -> bool {
        OffsetDateTime::now_utc() > self.expires_at
    }

    /// Check if request is still pending
    pub fn is_pending(&self) -> bool {
        self.status == PairingRequestStatus::Pending && !self.is_expired()
    }
}

/// Manager for pairing requests
#[derive(Clone)]
pub struct PairingRequestManager {
    requests: Arc<Mutex<HashMap<String, PairingRequest>>>,
    timeout_seconds: u64,
}

impl PairingRequestManager {
    /// Create a new pairing request manager
    pub fn new(timeout_seconds: u64) -> Self {
        Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
            timeout_seconds,
        }
    }

    /// Create a new pairing request
    pub fn create_request(
        &self,
        device_name: String,
        device_model: Option<String>,
        client_certificate: Vec<u8>,
        verification_code: String,
    ) -> Result<PairingRequest> {
        let request = PairingRequest::new(
            device_name,
            device_model,
            client_certificate,
            verification_code,
            self.timeout_seconds,
        );

        let request_id = request.request_id.clone();
        let mut requests = self.requests.lock().unwrap();

        // Clean up expired requests
        self.cleanup_expired_requests(&mut requests);

        requests.insert(request_id, request.clone());

        Ok(request)
    }

    /// Get a pairing request by ID
    pub fn get_request(&self, request_id: &str) -> Option<PairingRequest> {
        let mut requests = self.requests.lock().unwrap();

        // Check if expired and update status if needed
        let is_expired = requests.get(request_id).map(|r| r.is_expired()).unwrap_or(false);
        if is_expired {
            if let Some(req) = requests.get_mut(request_id) {
                req.status = PairingRequestStatus::Timeout;
            }
        }

        requests.get(request_id).cloned()
    }

    /// Approve a pairing request
    pub fn approve_request(&self, request_id: &str, client_id: String) -> Result<()> {
        let mut requests = self.requests.lock().unwrap();

        let request = requests
            .get_mut(request_id)
            .ok_or_else(|| anyhow::anyhow!("Pairing request not found"))?;

        if request.is_expired() {
            request.status = PairingRequestStatus::Timeout;
            anyhow::bail!("Pairing request has expired");
        }

        if request.status != PairingRequestStatus::Pending {
            anyhow::bail!("Pairing request is not pending");
        }

        request.status = PairingRequestStatus::Approved;
        request.client_id = Some(client_id);

        Ok(())
    }

    /// Reject a pairing request
    pub fn reject_request(&self, request_id: &str) -> Result<()> {
        let mut requests = self.requests.lock().unwrap();

        let request = requests
            .get_mut(request_id)
            .ok_or_else(|| anyhow::anyhow!("Pairing request not found"))?;

        if request.status != PairingRequestStatus::Pending {
            anyhow::bail!("Pairing request is not pending");
        }

        request.status = PairingRequestStatus::Rejected;

        Ok(())
    }

    /// Remove expired requests
    fn cleanup_expired_requests(&self, requests: &mut HashMap<String, PairingRequest>) {
        requests.retain(|_, request| !request.is_expired() || request.status != PairingRequestStatus::Pending);
    }

    /// Get number of pending requests
    pub fn pending_count(&self) -> usize {
        let requests = self.requests.lock().unwrap();
        requests.values().filter(|r| r.is_pending()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration as StdDuration;

    #[test]
    fn test_pairing_request_creation() {
        let request = PairingRequest::new(
            "Test Device".to_string(),
            Some("Test Model".to_string()),
            vec![1, 2, 3],
            "123-456".to_string(),
            60,
        );

        assert!(!request.request_id.is_empty());
        assert_eq!(request.device_name, "Test Device");
        assert_eq!(request.device_model, Some("Test Model".to_string()));
        assert_eq!(request.verification_code, "123-456");
        assert_eq!(request.status, PairingRequestStatus::Pending);
        assert!(!request.is_expired());
        assert!(request.is_pending());
    }

    #[test]
    fn test_pairing_request_expiry() {
        let request = PairingRequest::new(
            "Test Device".to_string(),
            None,
            vec![1, 2, 3],
            "123-456".to_string(),
            0, // Expires immediately
        );

        thread::sleep(StdDuration::from_millis(10));
        assert!(request.is_expired());
        assert!(!request.is_pending());
    }

    #[test]
    fn test_pairing_manager_create_request() {
        let manager = PairingRequestManager::new(60);

        let request = manager
            .create_request(
                "Test Device".to_string(),
                Some("Test Model".to_string()),
                vec![1, 2, 3],
                "123-456".to_string(),
            )
            .unwrap();

        assert!(!request.request_id.is_empty());
        assert_eq!(manager.pending_count(), 1);
    }

    #[test]
    fn test_pairing_manager_get_request() {
        let manager = PairingRequestManager::new(60);

        let request = manager
            .create_request(
                "Test Device".to_string(),
                None,
                vec![1, 2, 3],
                "123-456".to_string(),
            )
            .unwrap();

        let retrieved = manager.get_request(&request.request_id).unwrap();
        assert_eq!(retrieved.device_name, "Test Device");
        assert_eq!(retrieved.verification_code, "123-456");
    }

    #[test]
    fn test_pairing_manager_approve_request() {
        let manager = PairingRequestManager::new(60);

        let request = manager
            .create_request(
                "Test Device".to_string(),
                None,
                vec![1, 2, 3],
                "123-456".to_string(),
            )
            .unwrap();

        manager
            .approve_request(&request.request_id, "client-123".to_string())
            .unwrap();

        let retrieved = manager.get_request(&request.request_id).unwrap();
        assert_eq!(retrieved.status, PairingRequestStatus::Approved);
        assert_eq!(retrieved.client_id, Some("client-123".to_string()));
    }

    #[test]
    fn test_pairing_manager_reject_request() {
        let manager = PairingRequestManager::new(60);

        let request = manager
            .create_request(
                "Test Device".to_string(),
                None,
                vec![1, 2, 3],
                "123-456".to_string(),
            )
            .unwrap();

        manager.reject_request(&request.request_id).unwrap();

        let retrieved = manager.get_request(&request.request_id).unwrap();
        assert_eq!(retrieved.status, PairingRequestStatus::Rejected);
    }

    #[test]
    fn test_pairing_manager_expired_request() {
        let manager = PairingRequestManager::new(0); // Expires immediately

        let request = manager
            .create_request(
                "Test Device".to_string(),
                None,
                vec![1, 2, 3],
                "123-456".to_string(),
            )
            .unwrap();

        thread::sleep(StdDuration::from_millis(10));

        // Should not be able to approve expired request
        let result = manager.approve_request(&request.request_id, "client-123".to_string());
        assert!(result.is_err());

        let retrieved = manager.get_request(&request.request_id).unwrap();
        assert_eq!(retrieved.status, PairingRequestStatus::Timeout);
    }

    #[test]
    fn test_pairing_manager_cleanup() {
        let manager = PairingRequestManager::new(0); // Expires immediately

        // Create multiple expired requests
        manager.create_request("Device 1".to_string(), None, vec![1], "111-111".to_string()).unwrap();
        manager.create_request("Device 2".to_string(), None, vec![2], "222-222".to_string()).unwrap();

        thread::sleep(StdDuration::from_millis(10));

        // Create a new request, which should trigger cleanup
        let manager2 = PairingRequestManager::new(60);
        manager2.create_request("Device 3".to_string(), None, vec![3], "333-333".to_string()).unwrap();

        // Only the new request should be pending
        assert_eq!(manager2.pending_count(), 1);
    }
}
