use std::{
    convert::TryFrom,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    pin::Pin,
    sync::Arc,
};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use futures_util::Stream;
use quinn::{
    Connection, Endpoint, ReadExactError, RecvStream, SendStream, crypto::rustls::QuicClientConfig,
};
use rustls::{
    ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use serde_json;
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        TcpStream, lookup_host,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::{Mutex, mpsc},
};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, trace, warn};
use url::Url;
use webpki_roots::TLS_SERVER_ROOTS;

use super::{
    ControlConnectParams, ControlFrame, ControlSession, ControlSink, RelayTransport, TlsOptions,
    TunnelConnectParams,
};

const FRAME_HEADER_LEN: usize = 5;
const QUIC_ALPN: &[u8] = b"handcontrol-relay.v1";
const TUNNEL_BUFFER: usize = 16 * 1024;

#[derive(Debug, Default, Clone)]
pub struct QuicTransport;

#[async_trait]
impl RelayTransport for QuicTransport {
    async fn connect_control(&self, params: ControlConnectParams) -> Result<ControlSession> {
        let endpoint = Url::parse(&params.register_url)
            .with_context(|| format!("Invalid relay register URL {}", params.register_url))?;
        let host = endpoint
            .host_str()
            .ok_or_else(|| anyhow!("Relay register URL missing host component"))?
            .to_string();
        let port = resolve_quic_port(
            endpoint.port_or_known_default(),
            params.quic_port,
            endpoint.scheme(),
        )?;

        let parts = establish_quic_control(&host, port, params.tls).await?;
        Ok(build_control_session(parts))
    }

    async fn spawn_tunnel(&self, params: TunnelConnectParams) -> Result<()> {
        info!("Spawning tunnel {} via quic transport", params.tunnel_id);

        let url = Url::parse(&params.tunnel_url)
            .with_context(|| format!("Invalid relay tunnel URL {}", params.tunnel_url))?;
        let host = url
            .host_str()
            .ok_or_else(|| anyhow!("Relay tunnel URL missing host component"))?
            .to_string();
        let port = resolve_quic_port(url.port_or_known_default(), params.quic_port, url.scheme())?;

        let token = url
            .query_pairs()
            .find(|(key, _)| key == "token")
            .map(|(_, value)| value.to_string())
            .ok_or_else(|| anyhow!("Relay tunnel URL missing authentication token"))?;

        let endpoint = establish_quic_control(&host, port, params.tls).await?;
        spawn_server_tunnel(endpoint, params, token).await
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

    let state = Arc::new(QuicControlState {
        _endpoint: endpoint,
        _connection: connection,
        send: Arc::clone(&send),
    });

    let sink: Arc<dyn ControlSink> = Arc::new(QuicControlSink::new(Arc::clone(&state)));
    let stream = spawn_control_reader(state, recv);

    ControlSession::new(sink, stream)
}

fn spawn_control_reader(
    state: Arc<QuicControlState>,
    mut recv: RecvStream,
) -> Pin<Box<dyn Stream<Item = Result<ControlFrame>> + Send>> {
    let (tx, rx) = mpsc::channel::<Result<ControlFrame>>(32);

    tokio::spawn(async move {
        loop {
            match read_frame(&mut recv).await {
                Ok(Some(ControlFrame::Ping(payload))) => {
                    let mut guard = state.send.lock().await;
                    if let Err(err) =
                        write_frame(&mut *guard, FrameType::Pong, payload.as_slice()).await
                    {
                        warn!("Failed to respond to QUIC ping: {err}");
                        break;
                    }
                }
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

async fn establish_quic_control(
    host: &str,
    port: u16,
    tls: TlsOptions,
) -> Result<QuicConnectionParts> {
    let rustls_config = build_tls_config(tls)?;
    let quic_crypto = QuicClientConfig::try_from(rustls_config)
        .context("Failed to adapt rustls config for QUIC")?;
    let mut client_config = quinn::ClientConfig::new(Arc::new(quic_crypto));
    client_config.transport_config(Arc::new(quinn::TransportConfig::default()));

    let ipv6 = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0);
    let ipv4 = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
    let mut endpoint = Endpoint::client(ipv6).or_else(|_| Endpoint::client(ipv4))?;
    endpoint.set_default_client_config(client_config);

    let addresses: Vec<_> = lookup_host((host, port))
        .await
        .with_context(|| format!("Failed to resolve relay host {}:{}", host, port))?
        .collect();
    if addresses.is_empty() {
        bail!(
            "Relay host {}:{} did not resolve to any addresses",
            host,
            port
        );
    }

    let mut last_error: Option<anyhow::Error> = None;

    for addr in addresses {
        match endpoint.connect(addr, host) {
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

    Err(last_error.unwrap_or_else(|| anyhow!("QUIC connect attempt failed for {}", host)))
}

async fn spawn_server_tunnel(
    parts: QuicConnectionParts,
    params: TunnelConnectParams,
    token: String,
) -> Result<()> {
    let QuicConnectionParts {
        endpoint: _,
        connection: _,
        send,
        recv,
    } = parts;

    let ready = serde_json::json!({
        "type": "tunnel_ready",
        "tunnel_id": params.tunnel_id,
        "role": "server",
        "token": token,
    })
    .to_string();

    {
        let mut guard = send.lock().await;
        write_frame(&mut *guard, FrameType::Text, ready.as_bytes())
            .await
            .context("Failed to send tunnel_ready over QUIC")?;
    }

    let tcp_stream = TcpStream::connect(params.local_endpoint)
        .await
        .context("Failed to connect to local gRPC endpoint")?;
    let (tcp_read, tcp_write) = tcp_stream.into_split();

    let uplink = forward_tcp_to_quic(Arc::clone(&send), tcp_read, params.tunnel_id.clone());
    let downlink =
        forward_quic_to_tcp(Arc::clone(&send), recv, tcp_write, params.tunnel_id.clone());

    tokio::try_join!(uplink, downlink)?;
    Ok(())
}

async fn forward_tcp_to_quic(
    send: Arc<Mutex<SendStream>>,
    mut reader: OwnedReadHalf,
    tunnel_id: String,
) -> Result<()> {
    let mut buffer = vec![0u8; TUNNEL_BUFFER];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => {
                let mut guard = send.lock().await;
                let _ = write_frame(&mut *guard, FrameType::Close, &[]).await;
                break;
            }
            Ok(n) => {
                let mut guard = send.lock().await;
                if let Err(err) = write_frame(&mut *guard, FrameType::Binary, &buffer[..n]).await {
                    trace!(
                        tunnel = %tunnel_id,
                        error = %err,
                        "QUIC uplink write failed"
                    );
                    break;
                }
            }
            Err(err) => {
                trace!(
                    tunnel = %tunnel_id,
                    error = %err,
                    "Local TCP read failed"
                );
                break;
            }
        }
    }
    Ok(())
}

async fn forward_quic_to_tcp(
    send: Arc<Mutex<SendStream>>,
    mut recv: RecvStream,
    mut writer: OwnedWriteHalf,
    tunnel_id: String,
) -> Result<()> {
    loop {
        match read_frame(&mut recv).await {
            Ok(Some(ControlFrame::Binary(payload))) => {
                writer.write_all(&payload).await.with_context(|| {
                    format!(
                        "Failed to write QUIC payload to local TCP (tunnel {})",
                        tunnel_id
                    )
                })?;
            }
            Ok(Some(ControlFrame::Close)) | Ok(None) => break,
            Ok(Some(ControlFrame::Ping(payload))) => {
                let mut guard = send.lock().await;
                let _ = write_frame(&mut *guard, FrameType::Pong, payload.as_slice()).await;
            }
            Ok(Some(ControlFrame::Pong(_))) => {}
            Ok(Some(ControlFrame::Text(text))) => {
                warn!(
                    tunnel = %tunnel_id,
                    "Unexpected text frame on QUIC tunnel: {}",
                    text
                );
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
    Ok(())
}

struct QuicControlState {
    #[allow(dead_code)]
    _endpoint: Endpoint,
    #[allow(dead_code)]
    _connection: Connection,
    send: Arc<Mutex<SendStream>>,
}

struct QuicControlSink {
    state: Arc<QuicControlState>,
}

impl QuicControlSink {
    fn new(state: Arc<QuicControlState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ControlSink for QuicControlSink {
    async fn send_text(&self, message: &str) -> Result<()> {
        let mut guard = self.state.send.lock().await;
        write_frame(&mut *guard, FrameType::Text, message.as_bytes()).await
    }

    async fn send_binary(&self, payload: &[u8]) -> Result<()> {
        let mut guard = self.state.send.lock().await;
        write_frame(&mut *guard, FrameType::Binary, payload).await
    }

    async fn send_ping(&self) -> Result<()> {
        let mut guard = self.state.send.lock().await;
        write_frame(&mut *guard, FrameType::Ping, &[]).await
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

    let frame_type = match header[0] {
        0 => FrameType::Text,
        1 => FrameType::Binary,
        2 => FrameType::Close,
        3 => FrameType::Ping,
        4 => FrameType::Pong,
        other => bail!("Unknown QUIC frame type {}", other),
    };

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
            let text =
                String::from_utf8(payload).context("QUIC control frame contained invalid UTF-8")?;
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

fn resolve_quic_port(
    url_port: Option<u16>,
    configured_port: Option<u16>,
    scheme: &str,
) -> Result<u16> {
    if let Some(port) = configured_port {
        return Ok(port);
    }

    if let Some(port) = url_port {
        return Ok(port);
    }

    match scheme {
        "https" | "wss" => Ok(443),
        "http" | "ws" => Ok(80),
        other => bail!("Unsupported relay URL scheme for QUIC: {}", other),
    }
}
