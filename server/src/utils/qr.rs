use anyhow::{Context, Result};
use qr2term::print_qr;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// QR code payload for enrollment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentQrPayload {
    pub ips: Vec<String>,  // Changed from single ip (Breaking change)
    pub port: u16,
    pub cert_fingerprint: String,
    pub enrollment_token: String,
    pub server_id: String,
}

impl EnrollmentQrPayload {
    /// Create new QR payload
    pub fn new(
        ips: Vec<String>,
        port: u16,
        cert_fingerprint: String,
        enrollment_token: String,
        server_id: Uuid,
    ) -> Self {
        Self {
            ips,
            port,
            cert_fingerprint,
            enrollment_token,
            server_id: server_id.to_string(),
        }
    }

    /// Serialize to JSON
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).context("Failed to serialize QR payload to JSON")
    }

    /// Display QR code to terminal
    pub fn display_qr(&self) -> Result<()> {
        let json = self.to_json()?;

        println!("\n=== HandControl Enrollment QR Code ===\n");
        println!("Scan this QR code with your Android device to enroll:\n");

        print_qr(&json).context("Failed to generate QR code")?;

        // Display primary and alternative IPs
        if let Some(primary) = self.ips.first() {
            println!("\nPrimary Server: {}", primary);
        }

        if self.ips.len() > 1 {
            println!("Alternative IPs:");
            for ip in &self.ips[1..] {
                println!("  - {}", ip);
            }
        }

        println!("\nPort: {}", self.port);
        println!("Server ID: {}", self.server_id);
        println!("Token expires in 5 minutes");
        println!("\n======================================\n");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qr_payload_creation() {
        let server_id = Uuid::new_v4();
        let payload = EnrollmentQrPayload::new(
            vec!["192.168.1.100".to_string(), "10.0.0.1".to_string()],
            50051,
            "SHA256:abc123".to_string(),
            "token-uuid".to_string(),
            server_id,
        );

        assert_eq!(payload.ips, vec!["192.168.1.100", "10.0.0.1"]);
        assert_eq!(payload.port, 50051);
        assert_eq!(payload.cert_fingerprint, "SHA256:abc123");
        assert_eq!(payload.enrollment_token, "token-uuid");
        assert_eq!(payload.server_id, server_id.to_string());
    }

    #[test]
    fn test_qr_payload_json_serialization() {
        let server_id = Uuid::new_v4();
        let payload = EnrollmentQrPayload::new(
            vec!["192.168.1.100".to_string(), "10.0.0.1".to_string()],
            50051,
            "SHA256:abc123".to_string(),
            "token-uuid".to_string(),
            server_id,
        );

        let json = payload.to_json().unwrap();

        // Verify JSON contains all fields
        assert!(json.contains("\"ips\":[\"192.168.1.100\",\"10.0.0.1\"]"));
        assert!(json.contains("\"port\":50051"));
        assert!(json.contains("\"cert_fingerprint\":\"SHA256:abc123\""));
        assert!(json.contains("\"enrollment_token\":\"token-uuid\""));
        assert!(json.contains(&format!("\"server_id\":\"{}\"", server_id)));
    }

    #[test]
    fn test_qr_payload_json_roundtrip() {
        let server_id = Uuid::new_v4();
        let payload = EnrollmentQrPayload::new(
            vec!["192.168.1.100".to_string(), "10.0.0.1".to_string()],
            50051,
            "SHA256:abc123".to_string(),
            "token-uuid".to_string(),
            server_id,
        );

        let json = payload.to_json().unwrap();
        let deserialized: EnrollmentQrPayload = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.ips, payload.ips);
        assert_eq!(deserialized.port, payload.port);
        assert_eq!(deserialized.cert_fingerprint, payload.cert_fingerprint);
        assert_eq!(deserialized.enrollment_token, payload.enrollment_token);
        assert_eq!(deserialized.server_id, payload.server_id);
    }
}
