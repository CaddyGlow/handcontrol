use crate::storage::RegistryRelayInfo;
use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use http::HeaderValue;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::Mutex;
use tokio_tungstenite::{
    connect_async, connect_async_tls_with_config,
    tungstenite::{
        client::IntoClientRequest, error::ProtocolError, handshake::client::Response,
        Error as WsError, Message,
    },
    Connector,
};
use tracing::{debug, trace, warn};
use url::Url;
use uuid::Uuid;

const RELAY_PROTOCOL: &str = "handcontrol-relay.v1";
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const DUPLEX_BUFFER_SIZE: usize = 64 * 1024;

type RelayWebSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type RelaySink = futures_util::stream::SplitSink<RelayWebSocket, Message>;
type RelayStreamSplit = futures_util::stream::SplitStream<RelayWebSocket>;

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

#[derive(Debug, Serialize)]
struct TunnelReadyMessage<'a> {
    #[serde(rename = "type")]
    r#type: &'static str,
    tunnel_id: &'a str,
    role: &'static str,
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

#[derive(Debug, Deserialize)]
struct TunnelFailedMessage {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    r#type: String,
    reason: Option<String>,
}

/// Establish a relay tunnel and return a duplex stream that can be wrapped in TLS.
pub async fn establish_relay_tunnel(
    relay: &RegistryRelayInfo,
    server_id: Uuid,
    client_id: &Uuid,
) -> Result<RelayTunnel> {
    let connect_url = build_connect_url(&relay.relay_url)?;
    debug!(url = %connect_url, "Connecting to relay server");

    let pinned = parse_pinned_cert_sha256(relay.pinned_cert_sha256.as_deref())?;
    let connector = build_tls_connector(relay.allow_self_signed_tls, pinned)?;

    let (ws_stream, _response) = match connect_websocket(&connect_url, connector.clone(), true)
        .await
    {
        Ok(pair) => pair,
        Err(err) => {
            let should_retry = err
                .downcast_ref::<WsError>()
                .map_or(false, |ws_err| is_subprotocol_error(ws_err));

            if should_retry {
                warn!("Relay omitted Sec-WebSocket-Protocol; retrying without subprotocol header");
                connect_websocket(&connect_url, connector, false)
                    .await
                    .context("Relay rejected connection without subprotocol")?
            } else {
                return Err(err.context("Failed to establish WebSocket to relay"));
            }
        }
    };

    let (ws_tx_raw, mut ws_rx) = ws_stream.split();
    let ws_tx = Arc::new(Mutex::<RelaySink>::new(ws_tx_raw));

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
    {
        let mut guard = ws_tx.lock().await;
        guard
            .send(Message::Text(connect_json.into()))
            .await
            .context("Failed to send relay connect message")?;
    }

    let ack_message = ws_rx
        .next()
        .await
        .ok_or_else(|| anyhow!("Relay closed connection before connect_ack"))?;

    let ack = match ack_message {
        Ok(Message::Text(text)) => serde_json::from_str::<ConnectAck>(text.as_str())
            .context("Failed to parse connect_ack from relay")?,
        Ok(other) => {
            bail!("Relay returned unexpected frame before connect_ack: {other:?}");
        }
        Err(err) => {
            return Err(err).context("Relay connect_ack frame failed");
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
        .as_ref()
        .ok_or_else(|| anyhow!("Relay connect_ack missing tunnel_id"))?;

    let ready = TunnelReadyMessage {
        r#type: "tunnel_ready",
        tunnel_id,
        role: "client",
    };
    let ready_json = serde_json::to_string(&ready).context("Failed to serialize tunnel_ready")?;
    {
        let mut guard = ws_tx.lock().await;
        guard
            .send(Message::Text(ready_json.into()))
            .await
            .context("Failed to send tunnel_ready to relay")?;
    }

    let (client_stream, relay_stream) = io::duplex(DUPLEX_BUFFER_SIZE);
    let (relay_reader, relay_writer) = io::split(relay_stream);

    spawn_uplink(ws_tx.clone(), relay_reader, tunnel_id.to_string());
    spawn_downlink(ws_tx.clone(), ws_rx, relay_writer, tunnel_id.to_string());

    Ok(RelayTunnel {
        stream: client_stream,
        server_authority: ack.server_authority,
    })
}

fn is_subprotocol_error(err: &WsError) -> bool {
    matches!(
        err,
        WsError::Protocol(ProtocolError::SecWebSocketSubProtocolError(_))
    )
}

async fn connect_websocket(
    url: &Url,
    connector: Option<Connector>,
    include_protocol: bool,
) -> Result<(RelayWebSocket, Response), anyhow::Error> {
    let mut request = url
        .as_str()
        .into_client_request()
        .context("Failed to construct relay connect request")?;

    if include_protocol {
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            HeaderValue::from_static(RELAY_PROTOCOL),
        );
    }

    let result = match connector {
        Some(connector) => {
            connect_async_tls_with_config(request, None, false, Some(connector)).await
        }
        None => connect_async(request).await,
    };

    result.map_err(|err| anyhow!(err))
}

fn spawn_uplink(
    ws_tx: Arc<Mutex<RelaySink>>,
    mut relay_reader: ReadHalf<DuplexStream>,
    tunnel_id: String,
) {
    tokio::spawn(async move {
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            match relay_reader.read(&mut buffer).await {
                Ok(0) => {
                    trace!(tunnel = %tunnel_id, "Relay uplink reader reached EOF");
                    let _ = ws_tx.lock().await.send(Message::Close(None)).await;
                    break;
                }
                Ok(n) => {
                    let data = buffer[..n].to_vec();
                    if let Err(err) = ws_tx.lock().await.send(Message::binary(data)).await {
                        trace!(tunnel = %tunnel_id, error = %err, "Relay uplink send failed");
                        break;
                    }
                }
                Err(err) => {
                    trace!(tunnel = %tunnel_id, error = %err, "Relay uplink read failed");
                    let _ = ws_tx.lock().await.send(Message::Close(None)).await;
                    break;
                }
            }
        }
    });
}

fn spawn_downlink(
    ws_tx: Arc<Mutex<RelaySink>>,
    mut ws_rx: RelayStreamSplit,
    mut relay_writer: WriteHalf<DuplexStream>,
    tunnel_id: String,
) {
    tokio::spawn(async move {
        while let Some(frame) = ws_rx.next().await {
            match frame {
                Ok(Message::Binary(payload)) => {
                    if let Err(err) = relay_writer.write_all(payload.as_slice()).await {
                        trace!(tunnel = %tunnel_id, error = %err, "Relay downlink write failed");
                        break;
                    }
                }
                Ok(Message::Close(frame)) => {
                    debug!(tunnel = %tunnel_id, ?frame, "Relay downstream closed");
                    break;
                }
                Ok(Message::Ping(payload)) => {
                    let _ = ws_tx.lock().await.send(Message::Pong(payload)).await;
                }
                Ok(Message::Pong(_)) => {
                    trace!(tunnel = %tunnel_id, "Relay pong received");
                }
                Ok(Message::Text(text)) => {
                    if let Ok(msg) = serde_json::from_str::<TunnelFailedMessage>(text.as_str()) {
                        warn!(tunnel = %tunnel_id, reason = ?msg.reason, "Relay reported tunnel failure");
                        break;
                    } else {
                        warn!(tunnel = %tunnel_id, "Unexpected text frame in relay tunnel");
                    }
                }
                Ok(Message::Frame(_)) => {
                    trace!(tunnel = %tunnel_id, "Ignoring raw frame message from relay");
                }
                Err(err) => {
                    trace!(tunnel = %tunnel_id, error = %err, "Relay downlink frame failed");
                    break;
                }
            }
        }

        if let Err(err) = relay_writer.shutdown().await {
            trace!(tunnel = %tunnel_id, error = %err, "Relay writer shutdown failed");
        }
    });
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

fn build_tls_connector(
    allow_self_signed: bool,
    pinned_cert: Option<[u8; 32]>,
) -> Result<Option<Connector>> {
    if !allow_self_signed {
        return Ok(None);
    }

    #[derive(Debug)]
    struct AcceptSelfSignedVerifier {
        pinned: Option<[u8; 32]>,
    }

    impl rustls::client::danger::ServerCertVerifier for AcceptSelfSignedVerifier {
        fn verify_server_cert(
            &self,
            end_entity: &rustls::pki_types::CertificateDer<'_>,
            _intermediates: &[rustls::pki_types::CertificateDer<'_>],
            _server_name: &rustls::pki_types::ServerName<'_>,
            _ocsp_response: &[u8],
            _now: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            if let Some(expected) = self.pinned {
                let digest = Sha256::digest(end_entity.as_ref());
                if &digest[..] != expected.as_slice() {
                    return Err(rustls::Error::InvalidCertificate(
                        rustls::CertificateError::ApplicationVerificationFailure,
                    ));
                }
            }
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &rustls::pki_types::CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &rustls::pki_types::CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            vec![
                rustls::SignatureScheme::RSA_PKCS1_SHA256,
                rustls::SignatureScheme::RSA_PKCS1_SHA384,
                rustls::SignatureScheme::RSA_PKCS1_SHA512,
                rustls::SignatureScheme::RSA_PSS_SHA256,
                rustls::SignatureScheme::RSA_PSS_SHA384,
                rustls::SignatureScheme::RSA_PSS_SHA512,
                rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
                rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
                rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
                rustls::SignatureScheme::ED25519,
            ]
        }
    }

    let verifier = AcceptSelfSignedVerifier {
        pinned: pinned_cert,
    };
    let builder = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier));
    let mut config = builder.with_no_client_auth();
    config.alpn_protocols.push(b"h2".to_vec());
    Ok(Some(Connector::Rustls(Arc::new(config))))
}
