use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// Enrollment token for QR code pairing
#[derive(Debug, Clone)]
pub struct EnrollmentToken {
    pub token: String,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub used: bool,
}

impl EnrollmentToken {
    /// Create a new enrollment token
    pub fn new(ttl_seconds: u64) -> Self {
        let now = OffsetDateTime::now_utc();
        let token = Uuid::new_v4().to_string();

        Self {
            token,
            created_at: now,
            expires_at: now + Duration::seconds(ttl_seconds as i64),
            used: false,
        }
    }

    /// Check if token is expired
    pub fn is_expired(&self) -> bool {
        OffsetDateTime::now_utc() > self.expires_at
    }

    /// Check if token is valid (not expired, not used)
    pub fn is_valid(&self) -> bool {
        !self.used && !self.is_expired()
    }
}

/// Manager for enrollment tokens
#[derive(Clone)]
pub struct EnrollmentTokenManager {
    tokens: Arc<Mutex<HashMap<String, EnrollmentToken>>>,
    ttl_seconds: u64,
}

impl EnrollmentTokenManager {
    /// Create a new token manager
    pub fn new(ttl_seconds: u64) -> Self {
        Self {
            tokens: Arc::new(Mutex::new(HashMap::new())),
            ttl_seconds,
        }
    }

    /// Generate a new enrollment token
    pub fn generate_token(&self) -> Result<EnrollmentToken> {
        let token = EnrollmentToken::new(self.ttl_seconds);
        let token_str = token.token.clone();

        let mut tokens = self.tokens.lock().unwrap();
        tokens.insert(token_str, token.clone());

        // Clean up expired tokens
        self.cleanup_expired_tokens(&mut tokens);

        Ok(token)
    }

    /// Validate and consume a token
    pub fn validate_and_consume(&self, token_str: &str) -> Result<()> {
        let mut tokens = self.tokens.lock().unwrap();

        let token = tokens
            .get_mut(token_str)
            .ok_or_else(|| anyhow::anyhow!("Invalid enrollment token"))?;

        if token.used {
            anyhow::bail!("Enrollment token already used");
        }

        if token.is_expired() {
            tokens.remove(token_str);
            anyhow::bail!("Enrollment token expired");
        }

        // Mark as used
        token.used = true;

        Ok(())
    }

    /// Remove expired tokens
    fn cleanup_expired_tokens(&self, tokens: &mut HashMap<String, EnrollmentToken>) {
        tokens.retain(|_, token| !token.is_expired());
    }

    /// Get number of active tokens
    pub fn active_token_count(&self) -> usize {
        let tokens = self.tokens.lock().unwrap();
        tokens.values().filter(|t| t.is_valid()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration as StdDuration;

    #[test]
    fn test_enrollment_token_creation() {
        let token = EnrollmentToken::new(300);
        assert!(!token.token.is_empty());
        assert!(!token.is_expired());
        assert!(!token.used);
        assert!(token.is_valid());
    }

    #[test]
    fn test_enrollment_token_expiry() {
        let token = EnrollmentToken::new(0); // Expires immediately
        thread::sleep(StdDuration::from_millis(10));
        assert!(token.is_expired());
        assert!(!token.is_valid());
    }

    #[test]
    fn test_enrollment_token_used() {
        let mut token = EnrollmentToken::new(300);
        assert!(token.is_valid());

        token.used = true;
        assert!(!token.is_valid());
    }

    #[test]
    fn test_token_manager_generate() {
        let manager = EnrollmentTokenManager::new(300);
        let token = manager.generate_token().unwrap();

        assert!(!token.token.is_empty());
        assert!(token.is_valid());
        assert_eq!(manager.active_token_count(), 1);
    }

    #[test]
    fn test_token_manager_validate_and_consume() {
        let manager = EnrollmentTokenManager::new(300);
        let token = manager.generate_token().unwrap();

        // First validation should succeed
        assert!(manager.validate_and_consume(&token.token).is_ok());

        // Second validation should fail (already used)
        assert!(manager.validate_and_consume(&token.token).is_err());
    }

    #[test]
    fn test_token_manager_validate_invalid_token() {
        let manager = EnrollmentTokenManager::new(300);

        // Try to validate non-existent token
        assert!(manager.validate_and_consume("invalid-token").is_err());
    }

    #[test]
    fn test_token_manager_validate_expired_token() {
        let manager = EnrollmentTokenManager::new(0); // Expires immediately
        let token = manager.generate_token().unwrap();

        thread::sleep(StdDuration::from_millis(10));

        // Should fail because token is expired
        assert!(manager.validate_and_consume(&token.token).is_err());
    }

    #[test]
    fn test_token_manager_cleanup() {
        let manager = EnrollmentTokenManager::new(0); // Expires immediately

        // Generate multiple tokens
        manager.generate_token().unwrap();
        manager.generate_token().unwrap();
        manager.generate_token().unwrap();

        thread::sleep(StdDuration::from_millis(10));

        // All tokens should now be expired
        assert_eq!(manager.active_token_count(), 0);

        // Generate new token with longer TTL, should trigger cleanup of expired ones
        let manager2 = EnrollmentTokenManager::new(300);
        manager2.generate_token().unwrap();
        assert_eq!(manager2.active_token_count(), 1);
    }

    #[test]
    fn test_multiple_concurrent_tokens() {
        let manager = EnrollmentTokenManager::new(300);

        // Generate multiple tokens
        let token1 = manager.generate_token().unwrap();
        let token2 = manager.generate_token().unwrap();
        let token3 = manager.generate_token().unwrap();

        assert_eq!(manager.active_token_count(), 3);

        // Consume token2
        assert!(manager.validate_and_consume(&token2.token).is_ok());
        assert_eq!(manager.active_token_count(), 2);

        // token1 and token3 should still be valid
        assert!(manager.validate_and_consume(&token1.token).is_ok());
        assert!(manager.validate_and_consume(&token3.token).is_ok());

        assert_eq!(manager.active_token_count(), 0);
    }
}
