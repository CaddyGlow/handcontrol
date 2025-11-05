use anyhow::{Context, Result};
use ed25519_dalek::{
    SigningKey, VerifyingKey,
    pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey},
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayClaims {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub exp: u64,
    pub iat: u64,
    pub server_id: String,
    pub permissions: Vec<String>,
}

pub struct TokenIssuer {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
    server_id: Uuid,
}

impl TokenIssuer {
    /// Create a new TokenIssuer by loading or generating an Ed25519 keypair
    pub fn new(server_id: Uuid, key_path: &Path) -> Result<Self> {
        let signing_key = Self::load_or_generate_key(key_path)?;
        let verifying_key = VerifyingKey::from(&signing_key);

        Ok(Self {
            signing_key,
            verifying_key,
            server_id,
        })
    }

    /// Load existing key or generate a new one
    fn load_or_generate_key(key_path: &Path) -> Result<SigningKey> {
        if key_path.exists() {
            tracing::info!(path = %key_path.display(), "Loading existing Ed25519 key");
            Self::load_key(key_path)
        } else {
            tracing::info!(path = %key_path.display(), "Generating new Ed25519 key");
            let key = Self::generate_key()?;
            Self::save_key(&key, key_path)?;
            Ok(key)
        }
    }

    /// Load an existing Ed25519 private key from PEM
    fn load_key(path: &Path) -> Result<SigningKey> {
        let pem_content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read key file: {}", path.display()))?;

        SigningKey::from_pkcs8_pem(&pem_content)
            .context("Failed to parse Ed25519 private key from PEM")
    }

    /// Generate a new Ed25519 keypair
    fn generate_key() -> Result<SigningKey> {
        use rand::RngCore;
        let mut rng = rand::rng();
        let mut secret_bytes = [0u8; 32];
        rng.fill_bytes(&mut secret_bytes);
        Ok(SigningKey::from_bytes(&secret_bytes))
    }

    /// Save the Ed25519 private key to PEM format
    fn save_key(key: &SigningKey, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create key directory: {}", parent.display()))?;
        }

        let pem = key
            .to_pkcs8_pem(base64ct::LineEnding::LF)
            .context("Failed to encode key as PEM")?;

        fs::write(path, pem.as_bytes())
            .with_context(|| format!("Failed to write key file: {}", path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            fs::set_permissions(path, perms).with_context(|| {
                format!("Failed to set permissions on key file: {}", path.display())
            })?;
        }

        Ok(())
    }

    /// Generate a relay JWT token for a client
    pub fn generate_relay_token(
        &self,
        client_id: &str,
        relay_url: &str,
        ttl: Duration,
    ) -> Result<String> {
        use std::time::{SystemTime, UNIX_EPOCH};

        tracing::debug!(
            client_id = %client_id,
            relay_url = %relay_url,
            ttl_seconds = %ttl.as_secs(),
            "Generating relay JWT token"
        );

        if ttl.is_zero() {
            anyhow::bail!("Relay token TTL must be greater than zero");
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System time is before UNIX_EPOCH")?
            .as_secs();

        let ttl_secs = ttl.as_secs();
        let ttl_total = if ttl.subsec_nanos() > 0 {
            ttl_secs.saturating_add(1)
        } else {
            ttl_secs
        };

        if ttl_total == 0 {
            anyhow::bail!("Relay token TTL resolved to zero seconds");
        }

        let exp = now
            .checked_add(ttl_total)
            .context("Token expiry overflow")?;

        let claims = RelayClaims {
            iss: "handcontrol-server".to_string(),
            sub: client_id.to_string(),
            aud: relay_url.to_string(),
            exp,
            iat: now,
            server_id: self.server_id.to_string(),
            permissions: vec!["connect".to_string()],
        };

        tracing::trace!(
            server_id = %self.server_id,
            iat = %now,
            exp = %exp,
            ttl_seconds = %ttl_total,
            "JWT claims prepared"
        );

        let pem = self
            .signing_key
            .to_pkcs8_pem(base64ct::LineEnding::LF)
            .context("Failed to encode signing key as PEM")?;

        let encoding_key = EncodingKey::from_ed_pem(pem.as_bytes())
            .context("Failed to create encoding key from PEM")?;

        let token = encode(&Header::new(Algorithm::EdDSA), &claims, &encoding_key)
            .context("Failed to encode JWT token")?;

        tracing::debug!(
            client_id = %client_id,
            token_length = %token.len(),
            "JWT token generated successfully"
        );

        Ok(token)
    }

    /// Get the public key in PEM format
    pub fn public_key_pem(&self) -> Result<String> {
        self.verifying_key
            .to_public_key_pem(base64ct::LineEnding::LF)
            .context("Failed to encode public key as PEM")
    }

    /// Get the public key in base64 format (raw bytes)
    pub fn public_key_base64(&self) -> String {
        base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            self.verifying_key.as_bytes(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
    use tempfile::TempDir;

    #[test]
    fn test_generate_and_load_key() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("relay-key.pem");

        let server_id = Uuid::new_v4();

        // Generate new key
        let issuer1 = TokenIssuer::new(server_id, &key_path).unwrap();
        let pubkey1 = issuer1.public_key_base64();

        // Load existing key
        let issuer2 = TokenIssuer::new(server_id, &key_path).unwrap();
        let pubkey2 = issuer2.public_key_base64();

        // Should be the same key
        assert_eq!(pubkey1, pubkey2);
    }

    #[test]
    fn test_generate_token() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("relay-key.pem");

        let server_id = Uuid::new_v4();
        let issuer = TokenIssuer::new(server_id, &key_path).unwrap();

        let token = issuer
            .generate_relay_token(
                "test-client",
                "https://relay.example.com",
                Duration::from_secs(24 * 3600),
            )
            .unwrap();

        // Token should be a valid JWT (three parts separated by dots)
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
    }

    #[test]
    fn test_generate_token_respects_ttl() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("relay-key.pem");

        let server_id = Uuid::new_v4();
        let issuer = TokenIssuer::new(server_id, &key_path).unwrap();

        let relay_url = "https://relay.example.com";
        let token = issuer
            .generate_relay_token("client", relay_url, Duration::from_secs(300))
            .unwrap();

        let pubkey_pem = issuer.public_key_pem().unwrap();
        let decoding_key = DecodingKey::from_ed_pem(pubkey_pem.as_bytes()).unwrap();
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_audience(&[relay_url]);
        validation.validate_exp = false;

        let decoded = decode::<RelayClaims>(&token, &decoding_key, &validation).unwrap();
        let ttl = decoded.claims.exp - decoded.claims.iat;
        assert!((300..=301).contains(&ttl));
    }

    #[test]
    fn test_generate_token_zero_ttl_rejected() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("relay-key.pem");

        let server_id = Uuid::new_v4();
        let issuer = TokenIssuer::new(server_id, &key_path).unwrap();

        let result = issuer.generate_relay_token(
            "client",
            "https://relay.example.com",
            Duration::from_secs(0),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_public_key_formats() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("relay-key.pem");

        let server_id = Uuid::new_v4();
        let issuer = TokenIssuer::new(server_id, &key_path).unwrap();

        let pem = issuer.public_key_pem().unwrap();
        assert!(pem.starts_with("-----BEGIN PUBLIC KEY-----"));

        let base64 = issuer.public_key_base64();
        assert!(!base64.is_empty());
        // Ed25519 public keys are 32 bytes, base64 encoded should be 44 chars
        assert_eq!(base64.len(), 44);
    }
}
