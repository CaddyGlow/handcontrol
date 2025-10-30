pub mod client;
pub mod tokens;

use crate::config::parser::RelayConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub use client::RelayClient;
pub use tokens::TokenIssuer;

/// Payload sent over the control WebSocket immediately after `/register`.
#[derive(Debug, Serialize, PartialEq)]
pub struct RegisterMessage {
    #[serde(rename = "type")]
    pub r#type: &'static str,
    pub server_id: String,
    pub relay_secret: String,
    pub server_version: String,
    pub capabilities: Vec<String>,
    pub public_key: String,
    pub max_tunnels: Option<u32>,
}

/// Minimal acknowledgement returned by the relay after registration.
#[derive(Debug, Deserialize, PartialEq)]
pub struct RegisterAck {
    pub r#type: String,
    pub status: String,
    pub retry_after_seconds: Option<u32>,
}

/// Payload sent by the client when initiating a tunnel.
#[derive(Debug, Serialize, PartialEq)]
pub struct ConnectMessage {
    #[serde(rename = "type")]
    pub r#type: &'static str,
    pub server_id: String,
    pub relay_token: String,
    pub client_id: String,
    pub client_version: String,
}

/// Constructs the JSON value for a register payload based on runtime configuration.
pub fn build_register_payload(
    config: &RelayConfig,
    server_id: &Uuid,
    relay_secret: &str,
    public_key: &str,
    capabilities: &[&str],
    server_version: &str,
) -> Value {
    let message = RegisterMessage {
        r#type: "register",
        server_id: server_id.to_string(),
        relay_secret: relay_secret.to_string(),
        server_version: server_version.to_string(),
        capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
        public_key: public_key.to_string(),
        max_tunnels: config.max_relay_tunnels,
    };

    serde_json::to_value(message).expect("register message to serialize")
}

/// Convenience helper for building a JSON value for ConnectMessage.
pub fn build_connect_payload(
    server_id: &Uuid,
    relay_token: &str,
    client_id: &Uuid,
    client_version: &str,
) -> Value {
    let message = ConnectMessage {
        r#type: "connect",
        server_id: server_id.to_string(),
        relay_token: relay_token.to_string(),
        client_id: client_id.to_string(),
        client_version: client_version.to_string(),
    };

    serde_json::to_value(message).expect("connect message to serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_payload_includes_capabilities_and_limits() {
        let mut config = RelayConfig::default();
        config.max_relay_tunnels = Some(4);

        let server_id = Uuid::nil();
        let payload = build_register_payload(
            &config,
            &server_id,
            "secret",
            "pubkey",
            &["relay.v1"],
            "1.2.3",
        );

        assert_eq!(payload["type"], "register");
        assert_eq!(payload["server_id"], server_id.to_string());
        assert_eq!(payload["relay_secret"], "secret");
        assert_eq!(payload["server_version"], "1.2.3");
        assert_eq!(payload["capabilities"], serde_json::json!(["relay.v1"]));
        assert_eq!(payload["public_key"], "pubkey");
        assert_eq!(payload["max_tunnels"], 4);
    }

    #[test]
    fn connect_payload_contains_metadata() {
        let server_id = Uuid::nil();
        let client_id = Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeffffffff").unwrap();

        let payload = build_connect_payload(&server_id, "token", &client_id, "android-1.0");

        assert_eq!(payload["type"], "connect");
        assert_eq!(payload["server_id"], server_id.to_string());
        assert_eq!(payload["relay_token"], "token");
        assert_eq!(payload["client_id"], client_id.to_string());
        assert_eq!(payload["client_version"], "android-1.0");
    }

    #[test]
    fn register_ack_deserializes() {
        let json = r#"{
            "type": "register_ack",
            "status": "ok",
            "retry_after_seconds": 0
        }"#;

        let ack: RegisterAck = serde_json::from_str(json).unwrap();
        assert_eq!(
            ack,
            RegisterAck {
                r#type: "register_ack".to_string(),
                status: "ok".to_string(),
                retry_after_seconds: Some(0)
            }
        );
    }
}
