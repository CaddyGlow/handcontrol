use crate::{
    certificates::CertificatePaths,
    config,
    grpc_client::{
        connect_unauthenticated, connect_unauthenticated_via_relay, connect_unverified,
        persist_fingerprint,
    },
    storage::{RegistryRelayInfo, ServerRegistry, ServerRegistryEntry},
};
use anyhow::{anyhow, bail, Context, Result};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::convert::TryFrom;
use std::time::Duration;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::time::{sleep, Instant};
use tonic::{transport::Channel, Request};
use tracing::warn;
use uuid::Uuid;

#[derive(Debug)]
pub struct QrEnrollmentInput {
    pub payload: String,
    pub override_server_id: Option<Uuid>,
    pub device_name: Option<String>,
    pub device_model: Option<String>,
}

#[derive(Debug)]
pub struct QrEnrollmentOutcome {
    pub server_id: Uuid,
    pub client_id: Option<Uuid>,
    pub addresses: Vec<String>,
    pub port: u16,
    pub relay: Option<RegistryRelayInfo>,
    pub cert_directory: std::path::PathBuf,
    pub token_expiry: Option<OffsetDateTime>,
}

#[derive(Debug)]
pub struct ApprovalEnrollmentInput {
    pub addresses: Vec<String>,
    pub port: Option<u16>,
    pub server_id_hint: Option<Uuid>,
    pub device_name: Option<String>,
    pub device_model: Option<String>,
    pub timeout: std::time::Duration,
    pub poll_interval: std::time::Duration,
}

#[derive(Debug)]
pub struct ApprovalEnrollmentOutcome {
    pub server_id: Uuid,
    pub client_id: Option<Uuid>,
    pub addresses: Vec<String>,
    pub port: u16,
    pub relay: Option<RegistryRelayInfo>,
    pub cert_directory: std::path::PathBuf,
    pub verification_code: String,
}

#[derive(Debug, Deserialize)]
struct RawQrPayload {
    #[serde(default)]
    server_ip: Option<String>,
    #[serde(default, alias = "ips")]
    server_ips: Option<Vec<String>>,
    #[serde(default, alias = "port")]
    server_port: Option<u16>,
    #[serde(alias = "cert_fingerprint")]
    server_cert_fingerprint: String,
    server_id: String,
    enrollment_token: String,
    #[serde(default)]
    hostname: Option<String>,
    #[serde(default)]
    relay: Option<RawQrRelayInfo>,
    #[serde(default)]
    valid_until: Option<String>,
}

impl RawQrPayload {
    fn addresses(&self) -> Vec<String> {
        if let Some(ips) = &self.server_ips {
            if !ips.is_empty() {
                return ips.clone();
            }
        }
        let mut candidates = Vec::new();
        if let Some(ip) = &self.server_ip {
            candidates.push(ip.clone());
        }
        if let Some(host) = &self.hostname {
            candidates.push(host.clone());
        }
        candidates
    }

    fn port(&self) -> u16 {
        self.server_port.unwrap_or(50051)
    }
}

#[derive(Debug, Deserialize)]
struct RawQrRelayInfo {
    relay_url: String,
    relay_token: String,
    #[serde(default)]
    relay_required: bool,
    #[serde(default)]
    allow_self_signed_tls: bool,
    #[serde(default)]
    pinned_cert_sha256: Option<String>,
    #[serde(default)]
    quic_port: Option<u16>,
    #[serde(default)]
    quic_preferred: Option<bool>,
    #[serde(default)]
    transports: Vec<String>,
}

fn relay_from_qr(info: &RawQrRelayInfo) -> RegistryRelayInfo {
    RegistryRelayInfo {
        relay_url: info.relay_url.clone(),
        relay_token: info.relay_token.clone(),
        relay_required: info.relay_required,
        allow_self_signed_tls: info.allow_self_signed_tls,
        pinned_cert_sha256: info.pinned_cert_sha256.clone(),
        transports: info.transports.clone(),
        quic_port: info.quic_port,
        quic_preferred: info.quic_preferred,
    }
}

fn relay_from_proto(info: &crate::proto::RelayInfo) -> RegistryRelayInfo {
    RegistryRelayInfo {
        relay_url: info.relay_url.clone(),
        relay_token: info.relay_token.clone(),
        relay_required: info.relay_required,
        allow_self_signed_tls: false,
        pinned_cert_sha256: None,
        transports: info.transports.clone(),
        quic_port: info.quic_port.map(|p| p as u16),
        quic_preferred: info.quic_preferred,
    }
}

pub async fn enroll_via_qr(input: QrEnrollmentInput) -> Result<QrEnrollmentOutcome> {
    let raw: RawQrPayload = serde_json::from_str(&input.payload)
        .context("Failed to parse QR enrollment payload JSON")?;

    let token_expiry = raw
        .valid_until
        .as_ref()
        .map(|valid_until| {
            OffsetDateTime::parse(valid_until, &Rfc3339)
                .context("Invalid valid_until timestamp in QR payload")
        })
        .transpose()?;

    let server_id = if let Some(id) = input.override_server_id {
        id
    } else {
        Uuid::parse_str(&raw.server_id).context("Invalid server_id in payload")?
    };

    let mut addresses: Vec<String> = Vec::new();
    for candidate in raw.addresses() {
        if !addresses.iter().any(|existing| existing == &candidate) {
            addresses.push(candidate);
        }
    }
    if addresses.is_empty() {
        bail!("Enrollment payload did not include any server addresses");
    }

    let device_name = resolve_device_name(input.device_name.clone());
    let cert_paths = CertificatePaths::for_server(&server_id)?;
    let (client_cert_der, cert_pem, key_pem) =
        generate_client_certificate(Some(device_name.clone()), input.device_model.clone())?;

    let port = raw.port();
    let mut relay_info = raw.relay.as_ref().map(|info| relay_from_qr(info));
    let mut errors: Vec<String> = Vec::new();
    let mut last_err: Option<anyhow::Error> = None;

    for address in addresses.iter() {
        match attempt_qr_enrollment(
            address,
            port,
            &raw.server_cert_fingerprint,
            client_cert_der.clone(),
            &device_name,
            &raw.enrollment_token,
        )
        .await
        {
            Ok((client_id, response_relay)) => {
                return finalize_qr_enrollment(
                    &mut relay_info,
                    &cert_paths,
                    &cert_pem,
                    &key_pem,
                    server_id,
                    raw.hostname.as_ref(),
                    &addresses,
                    port,
                    &raw.server_cert_fingerprint,
                    client_id,
                    response_relay,
                    Some(address.as_str()),
                    token_expiry,
                );
            }
            Err(err) => {
                errors.push(format!("direct {}: {:#}", address, err));
                last_err = Some(err);
            }
        }
    }

    if let Some(relay_details) = relay_info.clone() {
        match attempt_qr_enrollment_via_relay(
            &relay_details,
            server_id,
            raw.hostname.as_deref(),
            &addresses,
            port,
            &raw.server_cert_fingerprint,
            client_cert_der.clone(),
            &device_name,
            &raw.enrollment_token,
        )
        .await
        {
            Ok((client_id, response_relay)) => {
                let merged_relay = response_relay.or(Some(relay_details));
                return finalize_qr_enrollment(
                    &mut relay_info,
                    &cert_paths,
                    &cert_pem,
                    &key_pem,
                    server_id,
                    raw.hostname.as_ref(),
                    &addresses,
                    port,
                    &raw.server_cert_fingerprint,
                    client_id,
                    merged_relay,
                    addresses.first().map(|s| s.as_str()),
                    token_expiry,
                );
            }
            Err(err) => {
                errors.push(format!("relay: {:#}", err));
                last_err = Some(err);
            }
        }
    }

    if !errors.is_empty() {
        bail!(
            "All enrollment attempts failed for server {}:\n{}",
            server_id,
            errors.join("\n")
        );
    }

    Err(last_err.unwrap_or_else(|| anyhow!("All enrollment attempts failed")))
}

pub async fn enroll_via_approval<F>(
    input: ApprovalEnrollmentInput,
    mut on_code: F,
) -> Result<ApprovalEnrollmentOutcome>
where
    F: FnMut(&str),
{
    if input.addresses.is_empty() {
        bail!("No server addresses provided for approval enrollment");
    }

    let device_name = resolve_device_name(input.device_name.clone());
    let (client_cert_der, cert_pem, key_pem) =
        generate_client_certificate(Some(device_name.clone()), input.device_model.clone())?;

    let port = input.port.unwrap_or(50051);
    let timeout = input.timeout;
    let poll_interval = input.poll_interval;

    let mut last_err: Option<anyhow::Error> = None;

    for address in input.addresses.iter() {
        match attempt_approval_enrollment(
            address,
            port,
            input.server_id_hint,
            &input.addresses,
            &client_cert_der,
            &cert_pem,
            &key_pem,
            &device_name,
            input.device_model.clone(),
            timeout,
            poll_interval,
            &mut on_code,
        )
        .await
        {
            Ok(outcome) => return Ok(outcome),
            Err(err) => {
                last_err = Some(err);
                continue;
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow!("All approval enrollment attempts failed")))
}

async fn attempt_qr_enrollment(
    address: &str,
    port: u16,
    server_fingerprint: &str,
    client_cert_der: Vec<u8>,
    device_name: &str,
    enrollment_token: &str,
) -> Result<(Option<Uuid>, Option<RegistryRelayInfo>)> {
    let mut client = connect_unauthenticated(address, port, server_fingerprint).await?;

    complete_enrollment_request(&mut client, client_cert_der, device_name, enrollment_token).await
}

async fn attempt_qr_enrollment_via_relay(
    relay_info: &RegistryRelayInfo,
    server_id: Uuid,
    hostname: Option<&str>,
    addresses: &[String],
    port: u16,
    server_fingerprint: &str,
    client_cert_der: Vec<u8>,
    device_name: &str,
    enrollment_token: &str,
) -> Result<(Option<Uuid>, Option<RegistryRelayInfo>)> {
    let transport_pref = config::load()
        .map(|cfg| cfg.network.relay.transport)
        .unwrap_or_else(|err| {
            warn!("Failed to load client config: {err:#}; defaulting to automatic transport");
            config::ClientConfig::default().network.relay.transport
        });
    let client_uuid = Uuid::parse_str(enrollment_token)
        .context("Enrollment token must be a UUID when using relay enrollment")?;
    let mut client = connect_unauthenticated_via_relay(
        relay_info,
        server_id,
        client_uuid,
        hostname,
        addresses,
        port,
        server_fingerprint,
        transport_pref,
    )
    .await?;

    complete_enrollment_request(&mut client, client_cert_der, device_name, enrollment_token).await
}

async fn attempt_approval_enrollment<F>(
    address: &str,
    port: u16,
    server_id_hint: Option<Uuid>,
    all_addresses: &[String],
    client_cert_der: &[u8],
    cert_pem: &str,
    key_pem: &str,
    device_name: &str,
    device_model: Option<String>,
    timeout: Duration,
    poll_interval: Duration,
    on_code: &mut F,
) -> Result<ApprovalEnrollmentOutcome>
where
    F: FnMut(&str),
{
    let (mut client, handshake_fingerprint) = connect_unverified(address, port).await?;

    let server_info = client
        .get_server_info(Request::new(crate::proto::ServerInfoRequest {}))
        .await
        .context("GetServerInfo RPC failed")?
        .into_inner();

    let server_id =
        Uuid::parse_str(&server_info.server_id).context("Server returned invalid server_id")?;

    if let Some(expected) = server_id_hint {
        if expected != server_id {
            bail!(
                "Server ID mismatch (expected {}, got {})",
                expected,
                server_id
            );
        }
    }

    let hostname = if server_info.hostname.is_empty() {
        None
    } else {
        Some(server_info.hostname.clone())
    };

    let pairing_request = crate::proto::RequestPairingRequest {
        device_name: device_name.to_string(),
        device_model: device_model.unwrap_or_default(),
        client_certificate: client_cert_der.to_vec(),
        verification_code: String::new(),
    };

    let response = client
        .request_pairing(Request::new(pairing_request))
        .await
        .context("RequestPairing RPC failed")?
        .into_inner();

    if response.pairing_request_id.is_empty() {
        bail!("Server returned empty pairing_request_id");
    }
    if response.server_cert_fingerprint.is_empty() {
        bail!("Server did not provide certificate fingerprint");
    }

    let server_cert_fp_bytes = response.server_cert_fingerprint.clone();
    let nonce = response.verification_nonce.clone();
    if nonce.is_empty() {
        bail!("Server did not provide verification nonce");
    }

    if handshake_fingerprint != server_cert_fp_bytes {
        bail!("Server fingerprint mismatch between TLS handshake and RPC response");
    }

    let local_code =
        compute_verification_code(client_cert_der, &server_cert_fp_bytes, &server_id, &nonce)?;

    if !response.verification_code.is_empty() && response.verification_code != local_code {
        bail!("Verification code mismatch between client and server");
    }

    on_code(&local_code);

    let server_timeout = if response.timeout_seconds > 0 {
        Duration::from_secs(response.timeout_seconds as u64)
    } else {
        timeout
    };
    let effective_deadline = Instant::now() + std::cmp::min(timeout, server_timeout);
    let poll_interval = if poll_interval.is_zero() {
        Duration::from_secs(1)
    } else {
        poll_interval
    };

    loop {
        let status = client
            .check_pairing_status(Request::new(crate::proto::CheckPairingStatusRequest {
                pairing_request_id: response.pairing_request_id.clone(),
            }))
            .await
            .context("CheckPairingStatus RPC failed")?
            .into_inner();

        let status_enum = crate::proto::PairingStatus::try_from(status.status)
            .unwrap_or(crate::proto::PairingStatus::Pending);

        match status_enum {
            crate::proto::PairingStatus::Approved => {
                let client_id = if status.client_id.is_empty() {
                    None
                } else {
                    Some(
                        Uuid::parse_str(&status.client_id)
                            .context("Server returned invalid client_id in approval")?,
                    )
                };

                let cert_paths = CertificatePaths::for_server(&server_id)?;
                cert_paths.write_client_credentials(cert_pem, key_pem)?;
                let fingerprint_str = format_fingerprint_bytes(&server_cert_fp_bytes);
                persist_fingerprint(&cert_paths, &fingerprint_str)?;

                let mut registry = ServerRegistry::load()?;
                let enrolled_at = OffsetDateTime::now_utc().format(&Rfc3339).ok();
                let relay_info = status
                    .relay_info
                    .as_ref()
                    .map(|info| relay_from_proto(info));

                registry.upsert(ServerRegistryEntry {
                    id: server_id,
                    hostname: hostname.clone(),
                    ip: Some(address.to_string()),
                    port: Some(port),
                    enrolled_at: enrolled_at.clone(),
                    last_seen: enrolled_at,
                    client_id,
                    cert_fingerprint: Some(fingerprint_str.clone()),
                    cert_path: Some(cert_paths.dir.to_string_lossy().into_owned()),
                    addresses: all_addresses.to_vec(),
                    relay: relay_info.clone(),
                });
                registry.save()?;

                return Ok(ApprovalEnrollmentOutcome {
                    server_id,
                    client_id,
                    addresses: all_addresses.to_vec(),
                    port,
                    relay: relay_info,
                    cert_directory: cert_paths.dir,
                    verification_code: local_code,
                });
            }
            crate::proto::PairingStatus::Rejected => {
                bail!("Pairing request was rejected by the server");
            }
            crate::proto::PairingStatus::Timeout => {
                bail!("Pairing request timed out on the server");
            }
            _ => {}
        }

        if Instant::now() >= effective_deadline {
            bail!("Timed out waiting for approval");
        }

        sleep(poll_interval).await;
    }
}

async fn complete_enrollment_request(
    client: &mut crate::proto::remote_control_client::RemoteControlClient<Channel>,
    client_cert_der: Vec<u8>,
    device_name: &str,
    enrollment_token: &str,
) -> Result<(Option<Uuid>, Option<RegistryRelayInfo>)> {
    let request = crate::proto::EnrollRequest {
        enrollment_token: enrollment_token.to_string(),
        client_certificate: client_cert_der,
        device_name: device_name.to_string(),
    };

    let response = client
        .enroll(Request::new(request))
        .await
        .context("Enroll RPC failed")?
        .into_inner();

    if !response.success {
        if !response.error_message.is_empty() {
            bail!("Enrollment failed: {}", response.error_message);
        }
        bail!("Enrollment failed (unspecified error)");
    }

    let client_id = if response.client_id.is_empty() {
        None
    } else {
        Some(Uuid::parse_str(&response.client_id).context("Invalid client_id returned by server")?)
    };
    let relay_info = response
        .relay_info
        .as_ref()
        .map(|info| relay_from_proto(info));

    Ok((client_id, relay_info))
}

fn finalize_qr_enrollment(
    relay_info: &mut Option<RegistryRelayInfo>,
    cert_paths: &CertificatePaths,
    cert_pem: &str,
    key_pem: &str,
    server_id: Uuid,
    hostname: Option<&String>,
    addresses: &[String],
    port: u16,
    server_fingerprint: &str,
    client_id: Option<Uuid>,
    new_relay: Option<RegistryRelayInfo>,
    connected_address: Option<&str>,
    token_expiry: Option<OffsetDateTime>,
) -> Result<QrEnrollmentOutcome> {
    if let Some(info) = new_relay {
        *relay_info = Some(info);
    }

    cert_paths.write_client_credentials(cert_pem, key_pem)?;
    persist_fingerprint(cert_paths, server_fingerprint)?;

    let mut registry = ServerRegistry::load()?;
    let enrolled_at = OffsetDateTime::now_utc().format(&Rfc3339).ok();
    let ip_field = connected_address.and_then(|addr| {
        let trimmed = addr.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let entry = ServerRegistryEntry {
        id: server_id,
        hostname: hostname.cloned(),
        ip: ip_field,
        port: Some(port),
        enrolled_at: enrolled_at.clone(),
        last_seen: enrolled_at,
        client_id,
        cert_fingerprint: Some(server_fingerprint.to_string()),
        cert_path: Some(cert_paths.dir.to_string_lossy().into_owned()),
        addresses: addresses.to_vec(),
        relay: relay_info.clone(),
    };
    registry.upsert(entry);
    registry.save()?;

    Ok(QrEnrollmentOutcome {
        server_id,
        client_id,
        addresses: addresses.to_vec(),
        port,
        relay: relay_info.clone(),
        cert_directory: cert_paths.dir.clone(),
        token_expiry,
    })
}

fn generate_client_certificate(
    device_name: Option<String>,
    device_model: Option<String>,
) -> Result<(Vec<u8>, String, String)> {
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    let common_name = device_name
        .clone()
        .unwrap_or_else(|| "HandControl CLI".to_string());
    dn.push(DnType::CommonName, &common_name);
    dn.push(DnType::OrganizationName, "HandControl");
    if let Some(model) = device_model {
        dn.push(DnType::OrganizationalUnitName, model);
    }
    params.distinguished_name = dn;
    params.not_after = rcgen::date_time_ymd(2035, 1, 1);

    let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
    let certificate = params.self_signed(&key_pair)?;
    let client_cert_der = certificate.der().as_ref().to_vec();
    let cert_pem = certificate.pem();
    let key_pem = key_pair.serialize_pem();
    Ok((client_cert_der, cert_pem, key_pem))
}

fn format_fingerprint_bytes(bytes: &[u8]) -> String {
    let hex = bytes
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    format!("SHA256:{hex}")
}

fn compute_verification_code(
    client_cert_der: &[u8],
    server_cert_fingerprint: &[u8],
    server_id: &Uuid,
    verification_nonce: &[u8],
) -> Result<String> {
    if verification_nonce.is_empty() {
        bail!("Verification nonce missing for verification code computation");
    }
    if server_cert_fingerprint.len() != 32 {
        bail!("Server fingerprint must be 32 bytes for verification code computation");
    }

    let client_hash = Sha256::digest(client_cert_der);

    let mut hasher = Sha256::new();
    hasher.update(&client_hash);
    hasher.update(server_cert_fingerprint);
    hasher.update(server_id.as_bytes());
    hasher.update(verification_nonce);

    let digest = hasher.finalize();
    let value = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    Ok(format!("{:03}-{:03}", value / 1_000, value % 1_000))
}

fn resolve_device_name(explicit: Option<String>) -> String {
    if let Some(name) = explicit {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    for key in ["HANDCONTROL_DEVICE_NAME", "USER", "USERNAME"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }

    if let Ok(host) = hostname::get() {
        let host_str = host.to_string_lossy().trim().to_string();
        if !host_str.is_empty() {
            return host_str;
        }
    }

    "HandControl CLI".to_string()
}
