use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use rustls::DigitallySignedStruct;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async, connect_async_tls_with_config,
    tungstenite::{
        Message, client::IntoClientRequest, http::HeaderValue, protocol::frame::Payload,
    },
};
use tracing::{error, info, trace};

use super::{
    ControlConnectParams, ControlFrame, ControlSession, ControlSink, RelayTransport, TlsOptions,
    TunnelConnectParams,
};

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

type LocalReadHalf = Box<dyn AsyncRead + Send + Unpin>;
type LocalWriteHalf = Box<dyn AsyncWrite + Send + Unpin>;

#[derive(Debug, Default, Clone)]
pub struct WebSocketTransport;

#[async_trait]
impl RelayTransport for WebSocketTransport {
    async fn connect_control(&self, params: ControlConnectParams) -> Result<ControlSession> {
        let request = build_relay_request(&params.register_url, &params.subprotocol)?;

        let (ws_stream, _) = if params.register_url.starts_with("wss://") {
            if let Some(connector) = build_tls_connector(params.tls)? {
                connect_async_tls_with_config(request, None, false, Some(connector))
                    .await
                    .context("Failed to connect to relay server")?
            } else {
                connect_async(request)
                    .await
                    .context("Failed to connect to relay server")?
            }
        } else {
            connect_async(request)
                .await
                .context("Failed to connect to relay server")?
        };

        let (sink, stream) = ws_stream.split();

        let shared_sink = Arc::new(Mutex::new(sink));
        let control_sink: Arc<dyn ControlSink> =
            Arc::new(WebSocketControlSink::new(shared_sink.clone()));

        let control_stream = stream.map(|msg| match msg {
            Ok(message) => map_control_frame(message),
            Err(err) => Err(anyhow!("WebSocket error: {}", err)),
        });

        Ok(ControlSession::new(control_sink, Box::pin(control_stream)))
    }

    async fn spawn_tunnel(&self, params: TunnelConnectParams) -> Result<()> {
        let TunnelConnectParams {
            tunnel_url,
            tunnel_id,
            subprotocol,
            local_endpoint,
            tls,
            quic_port: _,
        } = params;

        info!("Spawning tunnel {} via websocket transport", tunnel_id);

        tokio::spawn(async move {
            if let Err(err) = handle_tunnel(
                tunnel_url,
                tunnel_id.clone(),
                local_endpoint,
                tls,
                subprotocol,
            )
            .await
            {
                error!(%tunnel_id, "Tunnel task failed: {err:#}");
            }
        });

        Ok(())
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

struct WebSocketControlSink {
    sink: Arc<Mutex<futures_util::stream::SplitSink<WsStream, Message>>>,
}

impl WebSocketControlSink {
    fn new(sink: Arc<Mutex<futures_util::stream::SplitSink<WsStream, Message>>>) -> Self {
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

    async fn send_ping(&self) -> Result<()> {
        let mut guard = self.sink.lock().await;
        guard
            .send(Message::Ping(Payload::Vec(Vec::new())))
            .await
            .context("Failed to send control ping frame")
    }
}

fn payload_into_vec(payload: Payload) -> Vec<u8> {
    match payload {
        Payload::Owned(data) => data.to_vec(),
        Payload::Shared(data) => data.to_vec(),
        Payload::Vec(data) => data,
    }
}

async fn handle_tunnel(
    tunnel_url: String,
    tunnel_id: String,
    local_endpoint: SocketAddr,
    tls_options: TlsOptions,
    subprotocol: String,
) -> Result<()> {
    info!(%tunnel_id, "Opening tunnel to relay");

    let request = build_relay_request(&tunnel_url, &subprotocol)?;

    let (ws_stream, _) = if tunnel_url.starts_with("wss://") {
        if let Some(connector) = build_tls_connector(tls_options)? {
            connect_async_tls_with_config(request, None, false, Some(connector))
                .await
                .context("Failed to connect to tunnel endpoint")?
        } else {
            connect_async(request)
                .await
                .context("Failed to connect to tunnel endpoint")?
        }
    } else {
        connect_async(request)
            .await
            .context("Failed to connect to tunnel endpoint")?
    };

    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    let ready_msg = serde_json::json!({
        "type": "tunnel_ready",
        "tunnel_id": tunnel_id,
        "role": "server",
    });

    ws_sink
        .send(Message::Text(ready_msg.to_string().into()))
        .await
        .context("Failed to send tunnel_ready")?;

    info!(%tunnel_id, "Tunnel ready, connecting to local gRPC");

    let tcp_stream = TcpStream::connect(local_endpoint)
        .await
        .context("Failed to connect to local gRPC server")?;
    let (read_half, write_half) = tcp_stream.into_split();
    let mut local_read: LocalReadHalf = Box::new(read_half);
    let mut local_write: LocalWriteHalf = Box::new(write_half);

    info!(%tunnel_id, "Bridge established, forwarding data");

    let ws_to_local = async {
        while let Some(msg) = ws_stream.next().await {
            match msg {
                Ok(Message::Binary(payload)) => {
                    local_write
                        .write_all(payload.as_slice())
                        .await
                        .context("Failed to write to local TCP")?;
                }
                Ok(Message::Close(_)) => {
                    break;
                }
                Err(err) => {
                    return Err(anyhow!("WebSocket read error: {}", err));
                }
                Ok(other) => {
                    trace!(%tunnel_id, "Ignoring unexpected tunnel frame: {:?}", other);
                }
            }
        }
        Result::<_, anyhow::Error>::Ok(())
    };

    let local_to_ws = async {
        let mut buffer = vec![0u8; 8192];
        loop {
            match local_read.read(&mut buffer).await {
                Ok(0) => break,
                Ok(n) => {
                    ws_sink
                        .send(Message::Binary(Payload::Vec(buffer[..n].to_vec())))
                        .await
                        .context("Failed to send to WebSocket")?;
                }
                Err(err) => {
                    return Err(anyhow!("Local TCP read error: {}", err));
                }
            }
        }
        Result::<_, anyhow::Error>::Ok(())
    };

    tokio::select! {
        result = ws_to_local => result?,
        result = local_to_ws => result?,
    }

    info!(%tunnel_id, "Tunnel closed");
    Ok(())
}

fn build_relay_request(
    url: &str,
    subprotocol: &str,
) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request> {
    let mut request = url
        .into_client_request()
        .context("Failed to construct relay WebSocket request")?;
    let header = HeaderValue::from_str(subprotocol)
        .context("relay.websocket_subprotocol must be a valid header value")?;
    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", header);
    Ok(request)
}

fn build_tls_connector(tls_options: TlsOptions) -> Result<Option<Connector>> {
    if !tls_options.allow_self_signed {
        return Ok(None);
    }

    #[derive(Debug)]
    struct AcceptSelfSignedVerifier {
        pinned_cert_sha256: Option<[u8; 32]>,
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
            if let Some(expected) = self.pinned_cert_sha256 {
                use sha2::{Digest, Sha256};

                let digest = Sha256::digest(end_entity.as_ref());
                let digest_bytes: &[u8] = digest.as_ref();
                let expected_bytes: &[u8] = expected.as_ref();
                if digest_bytes != expected_bytes {
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
                rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
                rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
                rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
                rustls::SignatureScheme::RSA_PSS_SHA256,
                rustls::SignatureScheme::RSA_PSS_SHA384,
                rustls::SignatureScheme::RSA_PSS_SHA512,
                rustls::SignatureScheme::ED25519,
            ]
        }
    }

    let mut client_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(AcceptSelfSignedVerifier {
            pinned_cert_sha256: tls_options.pinned_cert_sha256,
        }))
        .with_no_client_auth();

    client_config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok(Some(Connector::Rustls(std::sync::Arc::new(client_config))))
}
