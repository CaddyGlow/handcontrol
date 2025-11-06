use crate::config::TransportPreference;
use crate::storage::RegistryRelayInfo;
use crate::transport::quic::QuicTransport;
use crate::transport::websocket::WebSocketTransport;
use crate::transport::{
    ControlConnectParams, ControlFrame, RelayTransport, TlsOptions, TunnelAttachParams,
};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::DuplexStream;
use tracing::debug;
use url::Url;
use uuid::Uuid;

const RELAY_PROTOCOL: &str = "handcontrol-relay.v1";
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Result of establishing a relay tunnel. Contains the virtual stream used for TLS and the
/// optional server authority advertised by the relay.
pub struct RelayTunnel {
    pub stream: DuplexStream,
    pub server_authority: Option<String>,
}

#[derive(Debug, Serialize)]
struct ConnectMessage<'a> {
    #[serde(rename = "type")]
    r#type: &'static str,
    server_id: &'a str,
    relay_token: &'a str,
    client_id: &'a str,
    client_version: &'static str,
}

#[derive(Debug, Deserialize)]
struct ConnectAck {
    #[serde(rename = "type")]
    r#type: String,
    status: String,
    tunnel_id: Option<String>,
    #[allow(dead_code)]
    relay_host: Option<String>,
    server_authority: Option<String>,
    error: Option<String>,
}
/// Establish a relay tunnel and return a duplex stream that can be wrapped in TLS.
pub async fn establish_relay_tunnel(
    relay: &RegistryRelayInfo,
    server_id: Uuid,
    client_id: &Uuid,
    transport_pref: TransportPreference,
) -> Result<RelayTunnel> {
    let connect_url = build_connect_url(&relay.relay_url)?;
    debug!(url = %connect_url, "Connecting to relay server");

    let pinned = parse_pinned_cert_sha256(relay.pinned_cert_sha256.as_deref())?;
    let tls_options = TlsOptions {
        allow_self_signed: relay.allow_self_signed_tls,
        pinned_cert_sha256: pinned,
    };

    let server_id_str = server_id.to_string();
    let client_id_str = client_id.to_string();
    let connect_payload = ConnectMessage {
        r#type: "connect",
        server_id: &server_id_str,
        relay_token: &relay.relay_token,
        client_id: &client_id_str,
        client_version: CLIENT_VERSION,
    };

    let connect_json = serde_json::to_string(&connect_payload)
        .context("Failed to serialize relay connect message")?;
    let connect_url_str = connect_url.as_str().to_owned();

    #[derive(Clone, Copy)]
    enum TransportKind {
        Websocket,
        Quic,
    }

    impl TransportKind {
        fn label(&self) -> &'static str {
            match self {
                TransportKind::Websocket => "websocket",
                TransportKind::Quic => "quic",
            }
        }
    }

    let attempt_order: Vec<TransportKind> = match transport_pref {
        TransportPreference::Auto => vec![TransportKind::Quic, TransportKind::Websocket],
        TransportPreference::Websocket => vec![TransportKind::Websocket],
        TransportPreference::Quic => vec![TransportKind::Quic, TransportKind::Websocket],
    };

    let mut errors: Vec<String> = Vec::new();

    for kind in attempt_order {
        let transport: Arc<dyn RelayTransport> = match kind {
            TransportKind::Websocket => Arc::new(WebSocketTransport::default()),
            TransportKind::Quic => Arc::new(QuicTransport::default()),
        };

        let control_params = ControlConnectParams {
            url: connect_url_str.clone(),
            subprotocol: match kind {
                TransportKind::Websocket => Some(RELAY_PROTOCOL.to_string()),
                TransportKind::Quic => None,
            },
            tls: tls_options,
            fallback_on_subprotocol_error: matches!(kind, TransportKind::Websocket),
        };

        let attempt_result: Result<RelayTunnel> = async {
            let mut session = transport
                .connect_control(control_params)
                .await
                .context("Failed to establish relay control connection")?;

            session
                .sink()
                .send_text(&connect_json)
                .await
                .context("Failed to send relay connect message")?;

            let ack_frame = session
                .next()
                .await
                .ok_or_else(|| anyhow!("Relay closed connection before connect_ack"))?
                .context("Relay connect_ack frame failed")?;

            let ack = match ack_frame {
                ControlFrame::Text(text) => serde_json::from_str::<ConnectAck>(text.as_str())
                    .context("Failed to parse connect_ack from relay")?,
                other => {
                    bail!("Relay returned unexpected frame before connect_ack: {other:?}");
                }
            };

            if ack.r#type != "connect_ack" {
                bail!("Relay response missing connect_ack (got {})", ack.r#type);
            }

            if ack.status != "ok" {
                let reason = ack.error.as_deref().unwrap_or("relay rejected connection");
                bail!("Relay rejected connection: {reason}");
            }

            let tunnel_id = ack
                .tunnel_id
                .clone()
                .ok_or_else(|| anyhow!("Relay connect_ack missing tunnel_id"))?;
            let server_authority = ack.server_authority.clone();

            let tunnel_connection = transport
                .attach_tunnel(
                    session,
                    TunnelAttachParams {
                        tunnel_id,
                        role: "client",
                    },
                )
                .await
                .context("Failed to attach relay tunnel")?;

            Ok(RelayTunnel {
                stream: tunnel_connection.stream,
                server_authority,
            })
        }
        .await;

        match attempt_result {
            Ok(tunnel) => return Ok(tunnel),
            Err(err) => {
                errors.push(format!("{} transport: {:#}", kind.label(), err));
            }
        }
    }

    if errors.is_empty() {
        bail!("Relay transport negotiation failed with no attempts executed");
    }

    bail!(
        "All relay transport attempts failed:\n{}",
        errors.join("\n")
    );
}

fn build_connect_url(base_url: &str) -> Result<Url> {
    let mut url = Url::parse(base_url).with_context(|| format!("Invalid relay URL: {base_url}"))?;

    let current_scheme = url.scheme().to_string();
    let scheme = match current_scheme.as_str() {
        "https" => "wss",
        "http" => "ws",
        "wss" | "ws" => current_scheme.as_str(),
        other => bail!("Unsupported relay URL scheme: {other}"),
    };

    if current_scheme != scheme {
        url.set_scheme(scheme)
            .map_err(|_| anyhow!("Failed to set relay URL scheme"))?;
    }

    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow!("Relay URL must be absolute"))?;
        segments.pop_if_empty();
        segments.push("connect");
    }

    url.set_query(None);
    Ok(url)
}

fn parse_pinned_cert_sha256(value: Option<&str>) -> Result<Option<[u8; 32]>> {
    if let Some(raw) = value {
        let normalized: String = raw
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != ':')
            .collect();

        if normalized.is_empty() {
            bail!("Pinned certificate fingerprint must not be empty");
        }

        let bytes =
            hex::decode(&normalized).context("Pinned certificate fingerprint must be valid hex")?;
        if bytes.len() != 32 {
            bail!("Pinned certificate fingerprint must decode to 32 bytes (SHA-256)");
        }

        let mut array = [0u8; 32];
        array.copy_from_slice(&bytes);
        Ok(Some(array))
    } else {
        Ok(None)
    }
}
