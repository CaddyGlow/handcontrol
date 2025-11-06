use std::{
    convert::TryFrom,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    pin::Pin,
    sync::Arc,
};

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use futures_util::{Stream, StreamExt};
use quinn::{
    crypto::rustls::QuicClientConfig, Connection, Endpoint, ReadExactError, RecvStream, SendStream,
};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use sha2::{Digest, Sha256};
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf},
    net::lookup_host,
    sync::{mpsc, Mutex},
};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{trace, warn};
use webpki_roots::TLS_SERVER_ROOTS;

use super::{
    ControlConnectParams, ControlFrame, ControlSession, ControlSink, RelayTransport, TlsOptions,
    TunnelAttachParams, TunnelConnection,
};

const DUPLEX_BUFFER_SIZE: usize = 64 * 1024;
const FRAME_HEADER_LEN: usize = 5;
const QUIC_ALPN: &[u8] = b"handcontrol-relay.v1";

#[derive(Debug, Default, Clone)]
pub struct QuicTransport;

#[async_trait]
impl RelayTransport for QuicTransport {
    async fn connect_control(&self, params: ControlConnectParams) -> Result<ControlSession> {
        let parts = establish_quic_control(&params).await?;
        Ok(build_control_session(parts))
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
            .context("Failed to send tunnel_ready over QUIC")?;

        let (client_stream, relay_stream) = io::duplex(DUPLEX_BUFFER_SIZE);
        let (relay_reader, relay_writer) = io::split(relay_stream);

        spawn_uplink(sink.clone(), relay_reader, tunnel_id.clone());
        spawn_downlink(sink, stream, relay_writer, tunnel_id);

        Ok(TunnelConnection {
            stream: client_stream,
        })
    }
}

struct QuicConnectionParts {
    endpoint: Endpoint,
    connection: Connection,
    send: Arc<Mutex<SendStream>>,
    recv: RecvStream,
}

fn build_control_session(parts: QuicConnectionParts) -> ControlSession {
    let QuicConnectionParts {
        endpoint,
        connection,
        send,
        recv,
    } = parts;

    let state = Arc::new(QuicConnectionState {
        _endpoint: endpoint,
        connection,
        send: Arc::clone(&send),
    });

    let sink: Arc<dyn ControlSink> = Arc::new(QuicControlSink::new(Arc::clone(&state)));
    let stream = spawn_control_reader(recv);

    ControlSession::new(sink, stream)
}

fn spawn_control_reader(
    mut recv: RecvStream,
) -> Pin<Box<dyn Stream<Item = Result<ControlFrame>> + Send>> {
    let (tx, rx) = mpsc::channel::<Result<ControlFrame>>(32);

    tokio::spawn(async move {
        loop {
            match read_frame(&mut recv).await {
                Ok(Some(frame)) => {
                    let is_close = matches!(frame, ControlFrame::Close);
                    if tx.send(Ok(frame)).await.is_err() {
                        break;
                    }
                    if is_close {
                        break;
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    let _ = tx.send(Err(err)).await;
                    break;
                }
            }
        }
    });

    Box::pin(ReceiverStream::new(rx))
}

async fn establish_quic_control(params: &ControlConnectParams) -> Result<QuicConnectionParts> {
    let rustls_config = build_tls_config(params.tls)?;
    let quic_crypto = QuicClientConfig::try_from(rustls_config)
        .context("Failed to adapt rustls config for QUIC")?;
    let mut client_config = quinn::ClientConfig::new(Arc::new(quic_crypto));
    client_config.transport_config(Arc::new(quinn::TransportConfig::default()));

    let ipv6 = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0);
    let ipv4 = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
    let mut endpoint = Endpoint::client(ipv6).or_else(|_| Endpoint::client(ipv4))?;
    endpoint.set_default_client_config(client_config);

    let addresses: Vec<_> = lookup_host((params.host.as_str(), params.port))
        .await
        .with_context(|| {
            format!(
                "Failed to resolve relay host {}:{}",
                params.host, params.port
            )
        })?
        .collect();
    if addresses.is_empty() {
        bail!(
            "Relay host {}:{} did not resolve to any addresses",
            params.host,
            params.port
        );
    }

    let mut last_error: Option<anyhow::Error> = None;

    for addr in addresses {
        match endpoint.connect(addr, &params.host) {
            Ok(connecting) => match connecting.await {
                Ok(connection) => match connection.open_bi().await {
                    Ok((send, recv)) => {
                        return Ok(QuicConnectionParts {
                            endpoint,
                            connection,
                            send: Arc::new(Mutex::new(send)),
                            recv,
                        });
                    }
                    Err(err) => {
                        last_error = Some(anyhow!(
                            "Failed to open QUIC control stream with {addr}: {err}"
                        ));
                    }
                },
                Err(err) => {
                    last_error = Some(anyhow!(
                        "Failed to complete QUIC handshake with {addr}: {err}"
                    ));
                }
            },
            Err(err) => {
                last_error = Some(anyhow!("Failed to initiate QUIC connect to {addr}: {err}"));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("QUIC connect attempt failed for {}", params.host)))
}

#[derive(Clone)]
struct QuicConnectionState {
    #[allow(dead_code)]
    _endpoint: Endpoint,
    connection: Connection,
    send: Arc<Mutex<SendStream>>,
}

struct QuicControlSink {
    state: Arc<QuicConnectionState>,
}

impl QuicControlSink {
    fn new(state: Arc<QuicConnectionState>) -> Self {
        Self { state }
    }

    async fn write_frame(&self, frame_type: FrameType, payload: &[u8]) -> Result<()> {
        let mut guard = self.state.send.lock().await;
        write_frame(&mut *guard, frame_type, payload).await
    }
}

#[async_trait]
impl ControlSink for QuicControlSink {
    async fn send_text(&self, message: &str) -> Result<()> {
        self.write_frame(FrameType::Text, message.as_bytes()).await
    }

    async fn send_binary(&self, payload: &[u8]) -> Result<()> {
        self.write_frame(FrameType::Binary, payload).await
    }

    async fn send_pong(&self, payload: &[u8]) -> Result<()> {
        self.write_frame(FrameType::Pong, payload).await
    }

    async fn send_close(&self) -> Result<()> {
        {
            let mut guard = self.state.send.lock().await;
            write_frame(&mut *guard, FrameType::Close, &[]).await?;
            guard
                .finish()
                .map_err(|err| anyhow!("Failed to finish QUIC send stream: {err}"))?;
        }

        self.state.connection.close(0u32.into(), b"close");
        Ok(())
    }
}

async fn read_frame(stream: &mut RecvStream) -> Result<Option<ControlFrame>> {
    let mut header = [0u8; FRAME_HEADER_LEN];
    match stream.read_exact(&mut header).await {
        Ok(()) => {}
        Err(ReadExactError::FinishedEarly(_)) => return Ok(None),
        Err(ReadExactError::ReadError(err)) => {
            return Err(anyhow!("Failed to read QUIC frame header: {err}"));
        }
    }

    let frame_type = FrameType::try_from(header[0])
        .map_err(|_| anyhow!("Received unknown QUIC control frame type {}", header[0]))?;
    let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;

    let mut payload = vec![0u8; length];
    if length > 0 {
        match stream.read_exact(&mut payload).await {
            Ok(()) => {}
            Err(ReadExactError::FinishedEarly(_)) => return Ok(None),
            Err(ReadExactError::ReadError(err)) => {
                return Err(anyhow!("Failed to read QUIC frame payload: {err}"));
            }
        }
    }

    let frame = match frame_type {
        FrameType::Text => {
            let text = String::from_utf8(payload)
                .map_err(|err| anyhow!("QUIC control frame contained invalid UTF-8: {err}"))?;
            ControlFrame::Text(text)
        }
        FrameType::Binary => ControlFrame::Binary(payload),
        FrameType::Close => ControlFrame::Close,
        FrameType::Ping => ControlFrame::Ping(payload),
        FrameType::Pong => ControlFrame::Pong(payload),
    };

    Ok(Some(frame))
}

async fn write_frame(stream: &mut SendStream, frame_type: FrameType, payload: &[u8]) -> Result<()> {
    if payload.len() > u32::MAX as usize {
        bail!("QUIC frame payload exceeds u32::MAX");
    }

    let mut header = [0u8; FRAME_HEADER_LEN];
    header[0] = frame_type as u8;
    header[1..5].copy_from_slice(&(payload.len() as u32).to_be_bytes());

    stream
        .write_all(&header)
        .await
        .context("Failed to write QUIC frame header")?;
    if !payload.is_empty() {
        stream
            .write_all(payload)
            .await
            .context("Failed to write QUIC frame payload")?;
    }
    stream
        .flush()
        .await
        .context("Failed to flush QUIC frame payload")
}

fn build_tls_config(tls: TlsOptions) -> Result<ClientConfig> {
    if tls.allow_self_signed {
        let verifier: Arc<dyn ServerCertVerifier> = Arc::new(SelfSignedVerifier {
            pinned: tls.pinned_cert_sha256,
        });
        let builder = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier);
        let mut config = builder.with_no_client_auth();
        config.alpn_protocols.push(QUIC_ALPN.to_vec());
        config.alpn_protocols.push(b"h3".to_vec());
        return Ok(config);
    }

    let mut roots = RootCertStore::empty();
    roots.extend(TLS_SERVER_ROOTS.iter().cloned());

    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols.push(QUIC_ALPN.to_vec());
    config.alpn_protocols.push(b"h3".to_vec());
    Ok(config)
}

#[derive(Debug)]
struct SelfSignedVerifier {
    pinned: Option<[u8; 32]>,
}

impl ServerCertVerifier for SelfSignedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if let Some(expected) = self.pinned {
            let digest = Sha256::digest(end_entity.as_ref());
            let mut actual = [0u8; 32];
            actual.copy_from_slice(&digest);
            if actual != expected {
                return Err(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::ApplicationVerificationFailure,
                ));
            }
        }

        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}

#[repr(u8)]
enum FrameType {
    Text = 0,
    Binary = 1,
    Close = 2,
    Ping = 3,
    Pong = 4,
}

impl TryFrom<u8> for FrameType {
    type Error = ();

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(FrameType::Text),
            1 => Ok(FrameType::Binary),
            2 => Ok(FrameType::Close),
            3 => Ok(FrameType::Ping),
            4 => Ok(FrameType::Pong),
            _ => Err(()),
        }
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
                    trace!(tunnel = %tunnel_id, "Relay QUIC uplink reader reached EOF");
                    let _ = sink.send_close().await;
                    break;
                }
                Ok(n) => {
                    if let Err(err) = sink.send_binary(&buffer[..n]).await {
                        trace!(
                            tunnel = %tunnel_id,
                            error = %err,
                            "Relay QUIC uplink send failed"
                        );
                        break;
                    }
                }
                Err(err) => {
                    trace!(
                        tunnel = %tunnel_id,
                        error = %err,
                        "Relay QUIC uplink read failed"
                    );
                    let _ = sink.send_close().await;
                    break;
                }
            }
        }
    });
}

fn spawn_downlink(
    sink: Arc<dyn ControlSink>,
    mut stream: Pin<Box<dyn Stream<Item = Result<ControlFrame>> + Send>>,
    mut relay_writer: WriteHalf<DuplexStream>,
    tunnel_id: String,
) {
    tokio::spawn(async move {
        while let Some(frame) = stream.next().await {
            match frame {
                Ok(ControlFrame::Binary(payload)) => {
                    if let Err(err) = relay_writer.write_all(payload.as_slice()).await {
                        trace!(
                            tunnel = %tunnel_id,
                            error = %err,
                            "Relay QUIC downlink write failed"
                        );
                        break;
                    }
                }
                Ok(ControlFrame::Close) => {
                    trace!(tunnel = %tunnel_id, "Relay QUIC downstream closed");
                    break;
                }
                Ok(ControlFrame::Ping(payload)) => {
                    let _ = sink.send_pong(payload.as_slice()).await;
                }
                Ok(ControlFrame::Pong(_)) => {}
                Ok(ControlFrame::Text(text)) => {
                    if let Ok(msg) = serde_json::from_str::<TunnelFailedMessage>(text.as_str()) {
                        warn!(
                            tunnel = %tunnel_id,
                            reason = ?msg.reason,
                            "Relay reported tunnel failure over QUIC"
                        );
                        break;
                    } else {
                        warn!(
                            tunnel = %tunnel_id,
                            "Unexpected text frame in relay QUIC tunnel"
                        );
                    }
                }
                Err(err) => {
                    trace!(
                        tunnel = %tunnel_id,
                        error = %err,
                        "Relay QUIC downlink frame failed"
                    );
                    break;
                }
            }
        }

        if let Err(err) = relay_writer.shutdown().await {
            trace!(
                tunnel = %tunnel_id,
                error = %err,
                "Relay QUIC writer shutdown failed"
            );
        }
    });
}

#[derive(Debug, serde::Serialize)]
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
