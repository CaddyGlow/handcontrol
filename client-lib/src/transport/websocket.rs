use std::{pin::Pin, sync::Arc};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use rustls::DigitallySignedStruct;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::{self, AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::Mutex;
use tokio_tungstenite::{
    connect_async, connect_async_tls_with_config,
    tungstenite::{
        client::IntoClientRequest, error::ProtocolError, http::HeaderValue,
        protocol::frame::Payload, Error as WsError, Message,
    },
    Connector, MaybeTlsStream, WebSocketStream,
};
use tracing::{debug, trace, warn};

use super::{
    ControlConnectParams, ControlFrame, ControlSession, ControlSink, RelayTransport, TlsOptions,
    TunnelAttachParams, TunnelConnection,
};

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
type WsSink = futures_util::stream::SplitSink<WsStream, Message>;

const DUPLEX_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Debug, Default, Clone)]
pub struct WebSocketTransport;

#[async_trait]
impl RelayTransport for WebSocketTransport {
    async fn connect_control(&self, params: ControlConnectParams) -> Result<ControlSession> {
        let attempts: Vec<Option<String>> =
            if params.subprotocol.is_some() && params.fallback_on_subprotocol_error {
                vec![params.subprotocol.clone(), None]
            } else {
                vec![params.subprotocol.clone()]
            };
        let mut last_err: Option<anyhow::Error> = None;

        for attempt in attempts {
            match connect_once(&params.url, attempt.as_deref(), params.tls).await {
                Ok(ws_stream) => {
                    return Ok(build_control_session(ws_stream));
                }
                Err(err) => {
                    let should_retry = params.fallback_on_subprotocol_error
                        && attempt.is_some()
                        && err
                            .downcast_ref::<WsError>()
                            .map_or(false, |ws_err| is_subprotocol_error(ws_err));

                    if should_retry {
                        last_err = Some(err);
                        continue;
                    } else {
                        return Err(err);
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            anyhow!(
                "Failed to establish relay control connection (subprotocol negotiation exhausted)"
            )
        }))
    }

    async fn attach_tunnel(
        &self,
        session: ControlSession,
        params: TunnelAttachParams,
    ) -> Result<TunnelConnection> {
        let TunnelAttachParams { tunnel_id, role } = params;
        let (sink, stream) = session.into_parts();

        let ready = TunnelReadyMessage {
            r#type: "tunnel_ready",
            tunnel_id: tunnel_id.as_str(),
            role,
        };

        let ready_json =
            serde_json::to_string(&ready).context("Failed to serialize tunnel_ready payload")?;

        sink.send_text(&ready_json)
            .await
            .context("Failed to send tunnel_ready to relay")?;

        let (client_stream, relay_stream) = io::duplex(DUPLEX_BUFFER_SIZE);
        let (relay_reader, relay_writer) = io::split(relay_stream);

        spawn_uplink(sink.clone(), relay_reader, tunnel_id.clone());
        spawn_downlink(sink, stream, relay_writer, tunnel_id);

        Ok(TunnelConnection {
            stream: client_stream,
        })
    }
}

fn build_control_session(ws_stream: WsStream) -> ControlSession {
    let (sink, stream) = ws_stream.split();
    let shared_sink = Arc::new(Mutex::new(sink));
    let control_sink: Arc<dyn ControlSink> = Arc::new(WebSocketControlSink::new(shared_sink));

    let control_stream = stream.map(|message| match message {
        Ok(msg) => map_control_frame(msg),
        Err(err) => Err(anyhow!("WebSocket error: {}", err)),
    });

    ControlSession::new(control_sink, Box::pin(control_stream))
}

struct WebSocketControlSink {
    sink: Arc<Mutex<WsSink>>,
}

impl WebSocketControlSink {
    fn new(sink: Arc<Mutex<WsSink>>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl ControlSink for WebSocketControlSink {
    async fn send_text(&self, message: &str) -> Result<()> {
        let mut guard = self.sink.lock().await;
        guard
            .send(Message::Text(message.to_owned().into()))
            .await
            .context("Failed to send control text frame")
    }

    async fn send_binary(&self, payload: &[u8]) -> Result<()> {
        let mut guard = self.sink.lock().await;
        guard
            .send(Message::Binary(payload.to_vec().into()))
            .await
            .context("Failed to send control binary frame")
    }

    async fn send_pong(&self, payload: &[u8]) -> Result<()> {
        let mut guard = self.sink.lock().await;
        guard
            .send(Message::Pong(Payload::Vec(payload.to_vec())))
            .await
            .context("Failed to send control pong frame")
    }

    async fn send_close(&self) -> Result<()> {
        let mut guard = self.sink.lock().await;
        guard
            .send(Message::Close(None))
            .await
            .context("Failed to send control close frame")
    }
}

fn map_control_frame(message: Message) -> Result<ControlFrame> {
    match message {
        Message::Text(payload) => Ok(ControlFrame::Text(payload.as_str().to_owned())),
        Message::Binary(payload) => Ok(ControlFrame::Binary(payload_into_vec(payload))),
        Message::Close(_) => Ok(ControlFrame::Close),
        Message::Ping(payload) => Ok(ControlFrame::Ping(payload_into_vec(payload))),
        Message::Pong(payload) => Ok(ControlFrame::Pong(payload_into_vec(payload))),
        other => Err(anyhow!("Unexpected WebSocket control frame: {:?}", other)),
    }
}

fn payload_into_vec(payload: Payload) -> Vec<u8> {
    match payload {
        Payload::Owned(data) => data.to_vec(),
        Payload::Shared(data) => data.to_vec(),
        Payload::Vec(data) => data,
    }
}

fn spawn_uplink(
    sink: Arc<dyn ControlSink>,
    mut relay_reader: ReadHalf<DuplexStream>,
    tunnel_id: String,
) {
    tokio::spawn(async move {
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            match relay_reader.read(&mut buffer).await {
                Ok(0) => {
                    trace!(tunnel = %tunnel_id, "Relay uplink reader reached EOF");
                    let _ = sink.send_close().await;
                    break;
                }
                Ok(n) => {
                    if let Err(err) = sink.send_binary(&buffer[..n]).await {
                        trace!(tunnel = %tunnel_id, error = %err, "Relay uplink send failed");
                        break;
                    }
                }
                Err(err) => {
                    trace!(tunnel = %tunnel_id, error = %err, "Relay uplink read failed");
                    let _ = sink.send_close().await;
                    break;
                }
            }
        }
    });
}

fn spawn_downlink(
    sink: Arc<dyn ControlSink>,
    mut stream: Pin<Box<dyn futures_util::Stream<Item = Result<ControlFrame>> + Send>>,
    mut relay_writer: WriteHalf<DuplexStream>,
    tunnel_id: String,
) {
    tokio::spawn(async move {
        while let Some(frame) = stream.next().await {
            match frame {
                Ok(ControlFrame::Binary(payload)) => {
                    if let Err(err) = relay_writer.write_all(payload.as_slice()).await {
                        trace!(tunnel = %tunnel_id, error = %err, "Relay downlink write failed");
                        break;
                    }
                }
                Ok(ControlFrame::Close) => {
                    debug!(tunnel = %tunnel_id, "Relay downstream closed");
                    break;
                }
                Ok(ControlFrame::Ping(payload)) => {
                    let _ = sink.send_pong(payload.as_slice()).await;
                }
                Ok(ControlFrame::Pong(payload)) => {
                    trace!(
                        tunnel = %tunnel_id,
                        size = payload.len(),
                        "Relay pong received"
                    );
                }
                Ok(ControlFrame::Text(text)) => {
                    if let Ok(msg) = serde_json::from_str::<TunnelFailedMessage>(text.as_str()) {
                        warn!(
                            tunnel = %tunnel_id,
                            reason = ?msg.reason,
                            "Relay reported tunnel failure"
                        );
                        break;
                    } else {
                        warn!(tunnel = %tunnel_id, "Unexpected text frame in relay tunnel");
                    }
                }
                Err(err) => {
                    trace!(tunnel = %tunnel_id, error = %err, "Relay downlink frame failed");
                    break;
                }
            }
        }

        if let Err(err) = relay_writer.shutdown().await {
            trace!(
                tunnel = %tunnel_id,
                error = %err,
                "Relay writer shutdown failed"
            );
        }
    });
}

#[derive(Debug, Serialize)]
struct TunnelReadyMessage<'a> {
    #[serde(rename = "type")]
    r#type: &'static str,
    tunnel_id: &'a str,
    role: &'a str,
}

#[derive(Debug, serde::Deserialize)]
struct TunnelFailedMessage {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    r#type: String,
    reason: Option<String>,
}

fn is_subprotocol_error(err: &WsError) -> bool {
    matches!(
        err,
        WsError::Protocol(ProtocolError::SecWebSocketSubProtocolError(_))
    )
}

async fn connect_once(url: &str, subprotocol: Option<&str>, tls: TlsOptions) -> Result<WsStream> {
    let mut request = url
        .into_client_request()
        .context("Failed to construct relay connect request")?;

    if let Some(proto) = subprotocol {
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", HeaderValue::from_str(proto)?);
    }

    let connector = build_tls_connector(tls)?;

    let (stream, _) = match connector {
        Some(connector) => {
            connect_async_tls_with_config(request, None, false, Some(connector)).await
        }
        None => connect_async(request).await,
    }
    .map_err(|err| anyhow!(err))?;

    Ok(stream)
}

fn build_tls_connector(tls: TlsOptions) -> Result<Option<Connector>> {
    if !tls.allow_self_signed {
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
            _dss: &DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &rustls::pki_types::CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
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
        pinned: tls.pinned_cert_sha256,
    };

    let builder = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier));
    let mut config = builder.with_no_client_auth();
    config.alpn_protocols.push(b"h2".to_vec());

    Ok(Some(Connector::Rustls(Arc::new(config))))
}
