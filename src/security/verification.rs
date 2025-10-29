use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Generate a 6-digit verification code from certificates and server ID
///
/// This code is used in approval mode pairing to prevent MITM attacks.
/// Both client and server compute the same code independently.
///
/// Algorithm:
/// 1. Compute SHA256 fingerprints of both certificates
/// 2. Combine: SHA256(client_fp || server_fp || server_id)
/// 3. Extract first 6 digits from hex representation
/// 4. Format as XXX-XXX for readability
pub fn generate_verification_code(
    client_cert_der: &[u8],
    server_cert_der: &[u8],
    server_id: &Uuid,
) -> String {
    // Compute fingerprints first
    let client_fingerprint = Sha256::digest(client_cert_der);
    let server_fingerprint = Sha256::digest(server_cert_der);

    // Combine fingerprints with server ID
    let mut hasher = Sha256::new();
    hasher.update(&client_fingerprint);
    hasher.update(&server_fingerprint);
    hasher.update(server_id.as_bytes());
    let hash = hasher.finalize();

    // Convert to hex and extract first 6 digits
    let hex = format!("{:x}", hash);
    let digits: String = hex.chars().filter(|c| c.is_ascii_digit()).take(6).collect();

    // If we don't have enough digits (unlikely), pad with zeros
    let digits = if digits.len() < 6 {
        format!("{:0<6}", digits)
    } else {
        digits
    };

    // Format as XXX-XXX
    format!("{}-{}", &digits[0..3], &digits[3..6])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_code_format() {
        let client_cert = b"client certificate data";
        let server_cert = b"server certificate data";
        let server_id = Uuid::new_v4();

        let code = generate_verification_code(client_cert, server_cert, &server_id);

        // Should be formatted as XXX-XXX
        assert_eq!(code.len(), 7);
        assert_eq!(code.chars().nth(3), Some('-'));

        // All characters except dash should be digits
        for (i, c) in code.chars().enumerate() {
            if i == 3 {
                assert_eq!(c, '-');
            } else {
                assert!(c.is_ascii_digit());
            }
        }
    }

    #[test]
    fn test_verification_code_deterministic() {
        let client_cert = b"client certificate data";
        let server_cert = b"server certificate data";
        let server_id = Uuid::new_v4();

        // Generate code multiple times with same inputs
        let code1 = generate_verification_code(client_cert, server_cert, &server_id);
        let code2 = generate_verification_code(client_cert, server_cert, &server_id);
        let code3 = generate_verification_code(client_cert, server_cert, &server_id);

        // Should always produce the same code
        assert_eq!(code1, code2);
        assert_eq!(code2, code3);
    }

    #[test]
    fn test_verification_code_different_inputs() {
        let client_cert1 = b"client certificate 1";
        let client_cert2 = b"client certificate 2";
        let server_cert = b"server certificate data";
        let server_id = Uuid::new_v4();

        let code1 = generate_verification_code(client_cert1, server_cert, &server_id);
        let code2 = generate_verification_code(client_cert2, server_cert, &server_id);

        // Different client certs should produce different codes
        assert_ne!(code1, code2);
    }

    #[test]
    fn test_verification_code_server_cert_matters() {
        let client_cert = b"client certificate data";
        let server_cert1 = b"server certificate 1";
        let server_cert2 = b"server certificate 2";
        let server_id = Uuid::new_v4();

        let code1 = generate_verification_code(client_cert, server_cert1, &server_id);
        let code2 = generate_verification_code(client_cert, server_cert2, &server_id);

        // Different server certs should produce different codes
        assert_ne!(code1, code2);
    }

    #[test]
    fn test_verification_code_server_id_matters() {
        let client_cert = b"client certificate data";
        let server_cert = b"server certificate data";
        let server_id1 = Uuid::new_v4();
        let server_id2 = Uuid::new_v4();

        let code1 = generate_verification_code(client_cert, server_cert, &server_id1);
        let code2 = generate_verification_code(client_cert, server_cert, &server_id2);

        // Different server IDs should produce different codes
        assert_ne!(code1, code2);
    }

    #[test]
    fn test_verification_code_order_matters() {
        let cert1 = b"certificate 1";
        let cert2 = b"certificate 2";
        let server_id = Uuid::new_v4();

        // Swap client and server certs
        let code1 = generate_verification_code(cert1, cert2, &server_id);
        let code2 = generate_verification_code(cert2, cert1, &server_id);

        // Order matters, so codes should be different
        assert_ne!(code1, code2);
    }

    #[test]
    fn test_verification_code_with_real_certificates() {
        use crate::security::certificates::ServerCertificate;

        let cert1 = ServerCertificate::generate().unwrap();
        let cert2 = ServerCertificate::generate().unwrap();
        let server_id = Uuid::new_v4();

        let code1 = generate_verification_code(&cert1.cert_der, &cert2.cert_der, &server_id);
        let code2 = generate_verification_code(&cert1.cert_der, &cert2.cert_der, &server_id);

        // Should be consistent
        assert_eq!(code1, code2);

        // Should be properly formatted
        assert_eq!(code1.len(), 7);
        assert_eq!(code1.chars().nth(3), Some('-'));
    }
}
