use anyhow::{Context, Result};
use async_stream::try_stream;
use bytes::Bytes;
use futures_util::stream::Stream;
use socket2::{Domain, Protocol, Socket, Type};
use std::convert::TryFrom;
use std::net::SocketAddr;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration as StdDuration;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use tracing::{info, warn};
use uuid::Uuid;

use super::proto::remote_control_server::{RemoteControl, RemoteControlServer};
use super::proto::{
    ApprovePairingRequest, ApprovePairingResponse, CheckPairingStatusRequest,
    CheckPairingStatusResponse, ConfigUpdateNotification, EnrollRequest, EnrollResponse,
    GenerateEnrollmentQrRequest, GenerateEnrollmentQrResponse, GetConfigVersionRequest,
    GetConfigVersionResponse, ListCapabilitiesRequest, ListCapabilitiesResponse,
    ListPendingPairingsRequest, ListPendingPairingsResponse, PendingPairingInfo,
    RequestPairingRequest, RequestPairingResponse, ServerInfoRequest, ServerInfoResponse,
    SessionClientMessage, SessionServerMessage, WatchConfigUpdatesRequest,
};
use super::proto::{session_client_message, session_server_message};

use crate::capabilities::{
    CapabilityKind as CapabilityKindConfig, CapabilityMetadata as CapabilityMetadataRuntime,
    CapabilityParameter as CapabilityParameterRuntime, CapabilityRegistry,
    SessionMode as CapabilitySessionMode,
};
use crate::cli::approve::approve_pairing_request;
use crate::config::{Config, ConfigBroadcaster};
use crate::notifications::NotificationManager;
use crate::security::certificates::{ClientCertificate, ServerCertificate};
use crate::security::enrollment::EnrollmentTokenManager;
use crate::security::pairing::{PairingRequestManager, PairingRequestStatus};
use crate::security::tls::build_permissive_server_config;
use crate::security::verification::generate_verification_code;
// Network utilities (using qualified paths to avoid unused import warnings)
use crate::relay::tokens::{BINDING_TYPE_CLIENT_ID, BINDING_TYPE_ENROLLMENT_TOKEN};
use crate::sessions::manager::{
    AttachmentMetadata, SessionBroadcastEvent, SessionHandle, SessionManagerError,
};
use crate::sessions::{SessionClientEvent, SessionId, SessionServerEvent};
use crate::storage::clients::ClientStore;
use sha2::{Digest, Sha256};

/// gRPC service implementation
pub struct RemoteControlService {
    config: Arc<RwLock<Config>>,
    server_cert: Arc<ServerCertificate>,
    server_id: Uuid,
    client_store: Arc<Mutex<ClientStore>>,
    enrollment_manager: EnrollmentTokenManager,
    pairing_manager: PairingRequestManager,
    notification_manager: NotificationManager,
    config_version: Arc<AtomicU64>,
    config_broadcaster: Arc<ConfigBroadcaster>,
    last_config_update: Arc<AtomicU64>,
    token_issuer: Option<Arc<crate::relay::TokenIssuer>>,
    session_manager: crate::sessions::manager::SessionManager,
}

impl RemoteControlService {
    pub fn new(
        config: Arc<RwLock<Config>>,
        server_cert: Arc<ServerCertificate>,
        server_id: Uuid,
        client_store: Arc<Mutex<ClientStore>>,
        enrollment_manager: EnrollmentTokenManager,
        pairing_manager: PairingRequestManager,
        notification_manager: NotificationManager,
        config_version: Arc<AtomicU64>,
        config_broadcaster: Arc<ConfigBroadcaster>,
        last_config_update: Arc<AtomicU64>,
        token_issuer: Option<Arc<crate::relay::TokenIssuer>>,
    ) -> Self {
        Self {
            config,
            server_cert,
            server_id,
            client_store,
            enrollment_manager,
            pairing_manager,
            notification_manager,
            config_version,
            config_broadcaster,
            last_config_update,
            token_issuer,
            session_manager: crate::sessions::manager::SessionManager::new(),
        }
    }

    /// Generate relay info for enrollment responses
    fn generate_relay_info(
        &self,
        subject: &str,
        binding_type: &str,
        binding_value: &str,
        ttl_override: Option<StdDuration>,
    ) -> Option<super::proto::RelayInfo> {
        let config = self.config.read().unwrap();

        if !config.relay.enabled || !config.relay.include_in_enrollment {
            return None;
        }

        let token_issuer = self.token_issuer.as_ref()?;
        let relay_url = config.relay.relay_server_url.as_ref()?;
        let config_ttl_seconds = config.relay.relay_token_ttl_hours.saturating_mul(3600);
        let config_ttl = StdDuration::from_secs(config_ttl_seconds.max(1));
        let ttl = match ttl_override {
            Some(override_ttl) if override_ttl.is_zero() => {
                tracing::warn!(
                    "Skipping relay token generation for {}: override TTL is zero",
                    subject
                );
                return None;
            }
            Some(override_ttl) => std::cmp::min(override_ttl, config_ttl),
            None => config_ttl,
        };

        match token_issuer.generate_relay_token(
            subject,
            relay_url,
            ttl,
            binding_type,
            binding_value,
        ) {
            Ok(token) => {
                let binding_hash = {
                    let digest = Sha256::digest(binding_value.as_bytes());
                    let hex = hex::encode(digest);
                    hex.chars().take(16).collect::<String>()
                };
                tracing::info!(
                    server_id = %self.server_id,
                    relay_binding_type = binding_type,
                    relay_binding_hash = %binding_hash,
                    relay_ttl_secs = ttl.as_secs(),
                    "Issued relay token"
                );
                Some(super::proto::RelayInfo {
                    relay_url: relay_url.clone(),
                    relay_token: token,
                    relay_required: false,
                })
            }
            Err(e) => {
                tracing::error!(
                    "Failed to generate relay token for subject {} (binding_type={}, binding_value={}): {}",
                    subject,
                    binding_type,
                    binding_value,
                    e
                );
                None
            }
        }
    }

    /// Ensure the incoming request is associated with an enrolled TLS client certificate.
    fn ensure_enrolled_client<T>(&self, request: &Request<T>) -> Result<Option<String>, Status> {
        let require_cert = self.config.read().unwrap().security.require_client_cert;

        let certs = match request.peer_certs() {
            Some(certs) if !certs.is_empty() => certs,
            _ => {
                if require_cert {
                    return Err(Status::unauthenticated("Client TLS certificate required"));
                } else {
                    warn!("Legacy client connected without presenting a TLS certificate");
                    return Ok(None);
                }
            }
        };

        let certificate = certs
            .first()
            .ok_or_else(|| Status::unauthenticated("Client TLS certificate required"))?;

        let fingerprint = Sha256::digest(certificate.as_ref());
        let fingerprint_hex = hex::encode(fingerprint);

        let client_store = self.client_store.lock().unwrap();
        if client_store.is_authorized(&fingerprint_hex) {
            Ok(Some(fingerprint_hex))
        } else if require_cert {
            Err(Status::permission_denied(
                "Client certificate is not enrolled on this server",
            ))
        } else {
            warn!(
                "Legacy client presented unrecognized certificate fingerprint={}",
                fingerprint_hex
            );
            Ok(None)
        }
    }

    async fn handle_resume_session(
        &self,
        session_id_str: String,
        resume: super::proto::SessionResume,
        stream: tonic::Streaming<SessionClientMessage>,
        client_ip: Option<std::net::IpAddr>,
        fingerprint: Option<String>,
    ) -> Result<
        Response<tokio_stream::wrappers::ReceiverStream<Result<SessionServerMessage, Status>>>,
        Status,
    > {
        if session_id_str.is_empty() {
            return Err(Status::invalid_argument(
                "Resume request missing session identifier",
            ));
        }

        let session_id = SessionId::from_str(&session_id_str).map_err(|_| {
            Status::invalid_argument("Resume request contained invalid session identifier")
        })?;

        info!(
            "ResumeSession RPC called: session_id={} (IP: {}, fingerprint={})",
            session_id,
            client_ip
                .as_ref()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            fingerprint.clone().unwrap_or_else(|| "legacy".to_string())
        );

        let snapshot = self
            .session_manager
            .snapshot(&session_id)
            .map_err(status_from_session_error)?;

        let capability_id = snapshot.capability_id.clone();
        let session_mode = snapshot.session_mode;

        let event_receiver = self
            .session_manager
            .subscribe_events(&session_id)
            .map_err(status_from_session_error)?;

        let client_sender = self
            .session_manager
            .get_sender(&session_id)
            .ok_or_else(|| Status::not_found("Session not found"))?;

        let (_attachment, next_token) = self
            .session_manager
            .attach_with_token(
                &session_id,
                AttachmentMetadata::new(fingerprint.clone()),
                &resume.resume_token,
            )
            .map_err(status_from_session_error)?;

        let buffered_outputs = self
            .session_manager
            .buffered_output_since(
                &session_id,
                resume.last_output_sequence,
                resume.last_error_sequence,
            )
            .map_err(status_from_session_error)?;

        let (response_tx, response_rx) = tokio::sync::mpsc::channel(128);

        tokio::spawn(forward_session_events(
            session_id.clone(),
            capability_id.clone(),
            session_mode,
            event_receiver,
            response_tx.clone(),
        ));

        tokio::spawn(forward_client_events(
            self.session_manager.clone(),
            session_id.clone(),
            stream,
            client_sender,
        ));

        let session_id_string = session_id.to_string();

        for frame in &buffered_outputs {
            let event = SessionServerEvent::Output {
                data: frame.data.clone(),
                stderr: matches!(frame.stream, crate::sessions::OutputStream::Stderr),
                binary: frame.binary,
                timestamp_ms: frame.timestamp_ms,
            };

            let message = session_event_to_proto(
                &session_id_string,
                &capability_id,
                session_mode,
                event,
                Some(frame),
                None,
            );

            if response_tx.send(Ok(message)).await.is_err() {
                break;
            }
        }

        let resume_ack = SessionServerMessage {
            session_id: session_id_string,
            payload: Some(session_server_message::Payload::ResumeAck(
                super::proto::SessionResumeAck {
                    resume_token: next_token,
                    replay_complete: Some(true),
                    message: None,
                },
            )),
        };

        let _ = response_tx.send(Ok(resume_ack)).await;

        let stream = tokio_stream::wrappers::ReceiverStream::new(response_rx);
        Ok(Response::new(stream))
    }
}

fn status_from_session_error(err: SessionManagerError) -> Status {
    match err {
        SessionManagerError::NotFound => Status::not_found("session not found"),
        SessionManagerError::AlreadyAttached => {
            Status::resource_exhausted("session already has an active attachment")
        }
        SessionManagerError::NotAttached => {
            Status::failed_precondition("session is not currently attached")
        }
        SessionManagerError::InvalidResumeToken => {
            Status::permission_denied("invalid resume token")
        }
    }
}

#[tonic::async_trait]
impl RemoteControl for RemoteControlService {
    async fn enroll(
        &self,
        request: Request<EnrollRequest>,
    ) -> Result<Response<EnrollResponse>, Status> {
        // Extract client IP
        let client_ip = crate::utils::network::extract_client_ip_from_headers(
            request.metadata(),
            request.remote_addr(),
        );

        let req = request.into_inner();

        info!(
            "Enrollment request from device: {} (IP: {})",
            req.device_name,
            client_ip
                .as_ref()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );

        // Check if QR code enrollment is enabled
        let config = self.config.read().unwrap();
        if !config.security.enrollment.qr_code_enabled {
            warn!("QR code enrollment is disabled");
            return Ok(Response::new(EnrollResponse {
                success: false,
                client_id: String::new(),
                error_message: "QR code enrollment is disabled on this server".to_string(),
                relay_info: None,
            }));
        }

        // Validate enrollment token
        if let Err(e) = self
            .enrollment_manager
            .validate_and_consume(&req.enrollment_token)
        {
            warn!("Enrollment failed for device {}: {}", req.device_name, e);
            return Ok(Response::new(EnrollResponse {
                success: false,
                client_id: String::new(),
                error_message: format!("Invalid enrollment token: {}", e),
                relay_info: None,
            }));
        }

        // Validate client certificate
        if req.client_certificate.is_empty() {
            warn!(
                "Enrollment failed for device {}: missing client certificate",
                req.device_name
            );
            return Err(Status::invalid_argument("Client certificate is required"));
        }

        // Parse client certificate
        let client_cert = ClientCertificate::from_der(req.client_certificate);

        // Store client certificate
        let mut store = self.client_store.lock().unwrap();
        let client_id = match store.add_client(&client_cert, req.device_name.clone(), client_ip) {
            Ok(id) => id,
            Err(e) => {
                warn!(
                    "Failed to store client certificate for device {}: {}",
                    req.device_name, e
                );
                return Err(Status::internal(format!(
                    "Failed to store client certificate: {}",
                    e
                )));
            }
        };

        info!(
            "Device {} enrolled successfully with client_id={}",
            req.device_name, client_id
        );

        let relay_info =
            self.generate_relay_info(&client_id, BINDING_TYPE_CLIENT_ID, &client_id, None);

        Ok(Response::new(EnrollResponse {
            success: true,
            client_id,
            error_message: String::new(),
            relay_info,
        }))
    }

    async fn generate_enrollment_qr(
        &self,
        _request: Request<GenerateEnrollmentQrRequest>,
    ) -> Result<Response<GenerateEnrollmentQrResponse>, Status> {
        info!("GenerateEnrollmentQR RPC called");

        // Read config once and use it throughout
        let config = self.config.read().unwrap();

        // Check if QR code enrollment is enabled
        if !config.security.enrollment.qr_code_enabled {
            warn!("QR code enrollment is disabled");
            return Ok(Response::new(GenerateEnrollmentQrResponse {
                success: false,
                qr_payload: String::new(),
                server_ips: vec![],
                server_port: 0,
                server_cert_fingerprint: String::new(),
                server_id: String::new(),
                enrollment_token: String::new(),
                ttl_seconds: 0,
                error_message: "QR code enrollment is disabled on this server".to_string(),
                relay_info: None,
                valid_until: String::new(),
            }));
        }

        // Generate enrollment token
        let token = match self.enrollment_manager.generate_token() {
            Ok(t) => t,
            Err(e) => {
                warn!("Failed to generate enrollment token: {}", e);
                return Ok(Response::new(GenerateEnrollmentQrResponse {
                    success: false,
                    qr_payload: String::new(),
                    server_ips: vec![],
                    server_port: 0,
                    server_cert_fingerprint: String::new(),
                    server_id: String::new(),
                    enrollment_token: String::new(),
                    ttl_seconds: 0,
                    error_message: format!("Failed to generate enrollment token: {}", e),
                    relay_info: None,
                    valid_until: String::new(),
                }));
            }
        };

        // Determine server IPs for clients to connect to
        let server_ips =
            if config.server.bind_address == "0.0.0.0" || config.server.bind_address == "::" {
                // Server is bound to all interfaces, get all usable local IPs with network config
                let network_options = crate::utils::network::NetworkOptions {
                    include_link_local_ipv6: config.network.include_link_local,
                    prefer_stable_addresses: config.network.prefer_stable_addresses,
                    max_addresses: Some(config.network.max_advertised_addresses),
                };
                let ips = crate::utils::network::get_all_local_ips_with_options(&network_options);
                if ips.is_empty() {
                    vec!["127.0.0.1".to_string()]
                } else {
                    ips
                }
            } else {
                vec![config.server.bind_address.clone()]
            };
        let server_port = config.server.port as i32;

        // Create QR payload
        let now = time::OffsetDateTime::now_utc();
        let enrollment_ttl = if token.expires_at > now {
            StdDuration::try_from(token.expires_at - now).ok()
        } else {
            None
        };
        let relay_info_proto = self.generate_relay_info(
            &token.token,
            BINDING_TYPE_ENROLLMENT_TOKEN,
            &token.token,
            enrollment_ttl,
        );
        let relay_qr_info = relay_info_proto
            .as_ref()
            .map(|info| crate::utils::qr::RelayQrInfo {
                relay_url: info.relay_url.clone(),
                relay_token: info.relay_token.clone(),
                relay_required: info.relay_required,
                allow_self_signed_tls: config.relay.allow_self_signed_tls,
                pinned_cert_sha256: config.relay.pinned_cert_sha256.clone(),
            });

        let payload = crate::utils::qr::EnrollmentQrPayload::new(
            server_ips.clone(),
            config.server.port,
            self.server_cert.fingerprint_display(),
            token.token.clone(),
            self.server_id,
            token.expires_at,
            relay_qr_info,
        );

        let valid_until = payload.valid_until.clone();

        let qr_payload = match payload.to_json() {
            Ok(json) => json,
            Err(e) => {
                warn!("Failed to serialize QR payload: {}", e);
                return Ok(Response::new(GenerateEnrollmentQrResponse {
                    success: false,
                    qr_payload: String::new(),
                    server_ips: vec![],
                    server_port: 0,
                    server_cert_fingerprint: String::new(),
                    server_id: String::new(),
                    enrollment_token: String::new(),
                    ttl_seconds: 0,
                    error_message: format!("Failed to serialize QR payload: {}", e),
                    relay_info: None,
                    valid_until: String::new(),
                }));
            }
        };

        info!(
            "Generated enrollment token, expires in {} seconds",
            config.security.enrollment_token_ttl
        );

        Ok(Response::new(GenerateEnrollmentQrResponse {
            success: true,
            qr_payload,
            server_ips,
            server_port,
            server_cert_fingerprint: self.server_cert.fingerprint_display(),
            server_id: self.server_id.to_string(),
            enrollment_token: token.token,
            ttl_seconds: config.security.enrollment_token_ttl as i32,
            error_message: String::new(),
            relay_info: relay_info_proto,
            valid_until,
        }))
    }

    async fn request_pairing(
        &self,
        request: Request<RequestPairingRequest>,
    ) -> Result<Response<RequestPairingResponse>, Status> {
        // Extract client IP
        let client_ip = crate::utils::network::extract_client_ip_from_headers(
            request.metadata(),
            request.remote_addr(),
        );

        let req = request.into_inner();

        info!(
            "Pairing request from device: {} (IP: {})",
            req.device_name,
            client_ip
                .as_ref()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );

        // Read config once and use it throughout
        let config = self.config.read().unwrap();

        // Check if approval enrollment is enabled
        if !config.security.enrollment.approval_enabled {
            warn!("Approval mode enrollment is disabled");
            return Ok(Response::new(RequestPairingResponse {
                pending: false,
                pairing_request_id: String::new(),
                timeout_seconds: 0,
                verification_code: String::new(),
                server_cert_fingerprint: vec![],
                error_message: "Approval mode enrollment is disabled on this server".to_string(),
                verification_nonce: vec![],
            }));
        }

        // Validate client certificate
        if req.client_certificate.is_empty() {
            warn!(
                "Pairing request failed for device {}: missing client certificate",
                req.device_name
            );
            return Err(Status::invalid_argument("Client certificate is required"));
        }

        // Generate random nonce for this pairing attempt (prevents precomputation attacks)
        let mut nonce = [0u8; 32];
        use rand::RngCore;
        rand::rng().fill_bytes(&mut nonce);

        // Generate verification code (server's computation)
        let server_verification_code = generate_verification_code(
            &req.client_certificate,
            &self.server_cert.cert_der,
            &self.server_id,
            &nonce,
        );

        // Verify client's code matches server's (if client sent one)
        // Note: With nonce-based verification, client computes code AFTER receiving nonce from server
        // So this verification is now done on the client side (out-of-band verification)
        if !req.verification_code.is_empty() && req.verification_code != server_verification_code {
            warn!(
                "Pairing verification failed for device {}: code mismatch (possible MITM)",
                req.device_name
            );
            return Ok(Response::new(RequestPairingResponse {
                pending: false,
                pairing_request_id: String::new(),
                timeout_seconds: 0,
                verification_code: String::new(),
                server_cert_fingerprint: vec![],
                error_message: "Verification failed - possible MITM attack".to_string(),
                verification_nonce: vec![],
            }));
        }

        // Create pairing request
        let pairing_request = match self.pairing_manager.create_request(
            req.device_name.clone(),
            if req.device_model.is_empty() {
                None
            } else {
                Some(req.device_model.clone())
            },
            req.client_certificate.clone(),
            server_verification_code.clone(),
            client_ip,
        ) {
            Ok(request) => request,
            Err(e) => {
                warn!(
                    "Failed to create pairing request for device {}: {}",
                    req.device_name, e
                );
                return Err(Status::internal(format!(
                    "Failed to create pairing request: {}",
                    e
                )));
            }
        };

        info!(
            "Pairing request created for device {}: request_id={}",
            req.device_name, pairing_request.request_id
        );

        // Show OS notification if available
        if config.security.enrollment.approval_notification {
            match self.notification_manager.show_pairing_notification(
                &req.device_name,
                &server_verification_code,
                &pairing_request.request_id,
            ) {
                Ok(true) => {
                    info!("Pairing notification shown for device {}", req.device_name);
                }
                Ok(false) => {
                    warn!("Pairing notification not shown (provider unavailable)");
                }
                Err(e) => {
                    warn!("Failed to show pairing notification: {}", e);
                }
            }
        } else {
            info!(
                "Pairing notifications disabled. Device: {}",
                req.device_name
            );
        }

        Ok(Response::new(RequestPairingResponse {
            pending: true,
            pairing_request_id: pairing_request.request_id,
            timeout_seconds: config.security.enrollment.approval_timeout_seconds as i32,
            verification_code: server_verification_code,
            server_cert_fingerprint: self.server_cert.fingerprint.to_vec(),
            error_message: String::new(),
            verification_nonce: nonce.to_vec(),
        }))
    }

    async fn check_pairing_status(
        &self,
        request: Request<CheckPairingStatusRequest>,
    ) -> Result<Response<CheckPairingStatusResponse>, Status> {
        // Extract client IP
        let client_ip = crate::utils::network::extract_client_ip_from_headers(
            request.metadata(),
            request.remote_addr(),
        );

        let req = request.into_inner();

        info!(
            "CheckPairingStatus RPC called: request_id={} (IP: {})",
            req.pairing_request_id,
            client_ip
                .as_ref()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );

        // Get pairing request
        let pairing_request = match self.pairing_manager.get_request(&req.pairing_request_id) {
            Some(request) => request,
            None => {
                warn!("Pairing request not found: {}", req.pairing_request_id);
                return Ok(Response::new(CheckPairingStatusResponse {
                    status: super::proto::PairingStatus::Unspecified as i32,
                    client_id: String::new(),
                    error_message: "Pairing request not found".to_string(),
                    relay_info: None,
                }));
            }
        };

        // Convert status
        let (status, client_id, error_message) = match pairing_request.status {
            PairingRequestStatus::Pending => {
                if pairing_request.is_expired() {
                    (
                        super::proto::PairingStatus::Timeout as i32,
                        String::new(),
                        "Pairing request timed out".to_string(),
                    )
                } else {
                    (
                        super::proto::PairingStatus::Pending as i32,
                        String::new(),
                        String::new(),
                    )
                }
            }
            PairingRequestStatus::Approved => {
                info!(
                    "Pairing request {} approved, client_id={}",
                    req.pairing_request_id,
                    pairing_request
                        .client_id
                        .as_ref()
                        .unwrap_or(&"unknown".to_string())
                );
                (
                    super::proto::PairingStatus::Approved as i32,
                    pairing_request.client_id.unwrap_or_default(),
                    String::new(),
                )
            }
            PairingRequestStatus::Rejected => {
                info!("Pairing request {} rejected", req.pairing_request_id);
                (
                    super::proto::PairingStatus::Rejected as i32,
                    String::new(),
                    "Pairing request rejected by user".to_string(),
                )
            }
            PairingRequestStatus::Timeout => {
                info!("Pairing request {} timed out", req.pairing_request_id);
                (
                    super::proto::PairingStatus::Timeout as i32,
                    String::new(),
                    "Pairing request timed out".to_string(),
                )
            }
        };

        let relay_info =
            if status == super::proto::PairingStatus::Approved as i32 && !client_id.is_empty() {
                self.generate_relay_info(&client_id, BINDING_TYPE_CLIENT_ID, &client_id, None)
            } else {
                None
            };

        Ok(Response::new(CheckPairingStatusResponse {
            status,
            client_id,
            error_message,
            relay_info,
        }))
    }

    async fn approve_pairing(
        &self,
        request: Request<ApprovePairingRequest>,
    ) -> Result<Response<ApprovePairingResponse>, Status> {
        // Extract client IP
        let client_ip = crate::utils::network::extract_client_ip_from_headers(
            request.metadata(),
            request.remote_addr(),
        );

        let fingerprint = self.ensure_enrolled_client(&request)?;
        let req = request.into_inner();
        info!(
            "ApprovePairing RPC called for request_id={} (IP: {}, fingerprint={})",
            req.pairing_request_id,
            client_ip
                .as_ref()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            fingerprint.clone().unwrap_or_else(|| "legacy".to_string())
        );

        let result = {
            let mut client_store = self.client_store.lock().unwrap();
            approve_pairing_request(
                &req.pairing_request_id,
                &self.pairing_manager,
                &mut client_store,
            )
        };

        match result {
            Ok(client_id) => {
                info!(
                    "Pairing request {} approved via RPC",
                    req.pairing_request_id
                );
                Ok(Response::new(ApprovePairingResponse {
                    success: true,
                    client_id,
                    error_message: String::new(),
                }))
            }
            Err(err) => {
                warn!(
                    "Failed to approve pairing request {}: {}",
                    req.pairing_request_id, err
                );
                Ok(Response::new(ApprovePairingResponse {
                    success: false,
                    client_id: String::new(),
                    error_message: err.to_string(),
                }))
            }
        }
    }

    async fn list_pending_pairings(
        &self,
        request: Request<ListPendingPairingsRequest>,
    ) -> Result<Response<ListPendingPairingsResponse>, Status> {
        info!("ListPendingPairings RPC called");

        let _ = self.ensure_enrolled_client(&request)?;

        let pending_requests = self.pairing_manager.list_pending();

        let requests: Vec<PendingPairingInfo> = pending_requests
            .into_iter()
            .map(|req| {
                let now = time::OffsetDateTime::now_utc();
                let seconds_remaining = (req.expires_at - now).whole_seconds().max(0);

                PendingPairingInfo {
                    request_id: req.request_id,
                    device_name: req.device_name,
                    device_model: req.device_model.unwrap_or_default(),
                    verification_code: req.verification_code,
                    expires_at_unix: req.expires_at.unix_timestamp(),
                    seconds_remaining: seconds_remaining as i32,
                    ip_address: req.ip_address.map(|ip| ip.to_string()).unwrap_or_default(),
                }
            })
            .collect();

        info!("Found {} pending pairing request(s)", requests.len());

        Ok(Response::new(ListPendingPairingsResponse { requests }))
    }

    async fn get_server_info(
        &self,
        request: Request<ServerInfoRequest>,
    ) -> Result<Response<ServerInfoResponse>, Status> {
        // Extract client IP
        let client_ip = crate::utils::network::extract_client_ip_from_headers(
            request.metadata(),
            request.remote_addr(),
        );

        info!(
            "GetServerInfo RPC called (IP: {})",
            client_ip
                .as_ref()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );

        // Get hostname
        let hostname = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown".to_string());

        // Get OS
        let os = std::env::consts::OS.to_string();

        // Get version from Cargo.toml
        let version = env!("CARGO_PKG_VERSION").to_string();

        let response = ServerInfoResponse {
            server_id: self.server_id.to_string(),
            hostname,
            version,
            os,
        };

        Ok(Response::new(response))
    }

    async fn list_capabilities(
        &self,
        request: Request<ListCapabilitiesRequest>,
    ) -> Result<Response<ListCapabilitiesResponse>, Status> {
        let client_ip = crate::utils::network::extract_client_ip_from_headers(
            request.metadata(),
            request.remote_addr(),
        );

        info!(
            "ListCapabilities RPC called (IP: {})",
            client_ip
                .as_ref()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );

        let _ = self.ensure_enrolled_client(&request)?;

        let config_guard = self.config.read().unwrap();
        let registry = CapabilityRegistry::from_config(&config_guard).map_err(|e| {
            warn!("Failed to build capability registry: {}", e);
            Status::internal("Failed to load capabilities")
        })?;
        let config_version = self.config_version.load(Ordering::SeqCst);

        let capabilities = registry
            .list_metadata()
            .into_iter()
            .map(proto_from_metadata)
            .collect();

        Ok(Response::new(ListCapabilitiesResponse {
            capabilities,
            config_version,
        }))
    }

    type OpenSessionStream =
        tokio_stream::wrappers::ReceiverStream<Result<SessionServerMessage, Status>>;

    async fn open_session(
        &self,
        request: Request<tonic::Streaming<SessionClientMessage>>,
    ) -> Result<Response<Self::OpenSessionStream>, Status> {
        let client_ip = crate::utils::network::extract_client_ip_from_headers(
            request.metadata(),
            request.remote_addr(),
        );

        let fingerprint = self.ensure_enrolled_client(&request)?;
        let mut stream = request.into_inner();

        let initial_message = stream
            .message()
            .await?
            .ok_or_else(|| Status::invalid_argument("Missing initial session message"))?;

        let initial_session_id = initial_message.session_id.clone();

        match initial_message.payload {
            Some(session_client_message::Payload::Open(open)) => {
                info!(
                    "OpenSession RPC called: capability_id={} (IP: {}, fingerprint={})",
                    open.capability_id,
                    client_ip
                        .as_ref()
                        .map(|ip| ip.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    fingerprint.clone().unwrap_or_else(|| "legacy".to_string())
                );

                let config_snapshot = {
                    let guard = self.config.read().unwrap();
                    guard.clone()
                };
                let registry = CapabilityRegistry::from_config(&config_snapshot).map_err(|e| {
                    warn!("Failed to build capability registry: {}", e);
                    Status::internal("Failed to load capabilities")
                })?;

                let capability_handle = registry.get(&open.capability_id).ok_or_else(|| {
                    warn!("Capability not found: {}", open.capability_id);
                    Status::not_found(format!("Capability '{}' not found", open.capability_id))
                })?;

                let capability = capability_handle.capability();

                let session_handle = self
                    .session_manager
                    .open_session(capability, open.parameters.clone(), fingerprint.clone())
                    .await
                    .map_err(|e| {
                        warn!("Failed to open capability session: {}", e);
                        Status::internal("Failed to open capability session")
                    })?;

                let session_manager = self.session_manager.clone();
                let SessionHandle {
                    id: session_id,
                    capability_id,
                    metadata: _metadata,
                    session_mode,
                    client_sender,
                    event_receiver,
                    resume_token: _initial_resume_token,
                } = session_handle;

                session_manager
                    .attach(&session_id, AttachmentMetadata::new(fingerprint.clone()))
                    .map_err(status_from_session_error)?;

                let (response_tx, response_rx) = tokio::sync::mpsc::channel(128);

                tokio::spawn(forward_session_events(
                    session_id.clone(),
                    capability_id,
                    session_mode,
                    event_receiver,
                    response_tx.clone(),
                ));

                tokio::spawn(forward_client_events(
                    session_manager,
                    session_id,
                    stream,
                    client_sender,
                ));

                let stream = tokio_stream::wrappers::ReceiverStream::new(response_rx);
                Ok(Response::new(stream))
            }

            Some(session_client_message::Payload::Resume(resume)) => {
                self.handle_resume_session(
                    initial_session_id,
                    resume,
                    stream,
                    client_ip,
                    fingerprint,
                )
                .await
            }
            _ => {
                warn!("First message for OpenSession must be SessionOpen or SessionResume");
                Err(Status::invalid_argument(
                    "First message must be SessionOpen or SessionResume",
                ))
            }
        }
    }

    async fn get_config_version(
        &self,
        request: Request<GetConfigVersionRequest>,
    ) -> Result<Response<GetConfigVersionResponse>, Status> {
        // Extract client IP
        let client_ip = crate::utils::network::extract_client_ip_from_headers(
            request.metadata(),
            request.remote_addr(),
        );

        info!(
            "GetConfigVersion RPC called (IP: {})",
            client_ip
                .as_ref()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );

        let _ = self.ensure_enrolled_client(&request)?;

        let config_version = self.config_version.load(Ordering::SeqCst);
        let last_updated_ms = self.last_config_update.load(Ordering::SeqCst) as i64;

        info!(
            "Returning config version {} (last updated: {}ms)",
            config_version, last_updated_ms
        );

        Ok(Response::new(GetConfigVersionResponse {
            config_version,
            last_updated_ms,
        }))
    }

    type WatchConfigUpdatesStream =
        tokio_stream::wrappers::ReceiverStream<Result<ConfigUpdateNotification, Status>>;

    async fn watch_config_updates(
        &self,
        request: Request<WatchConfigUpdatesRequest>,
    ) -> Result<Response<Self::WatchConfigUpdatesStream>, Status> {
        // Extract client IP
        let client_ip = crate::utils::network::extract_client_ip_from_headers(
            request.metadata(),
            request.remote_addr(),
        );

        info!(
            "WatchConfigUpdates RPC called - starting config update stream (IP: {})",
            client_ip
                .as_ref()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );

        let _ = self.ensure_enrolled_client(&request)?;

        // Subscribe to config updates
        let mut rx = self.config_broadcaster.subscribe();

        // Create channel for streaming updates
        let (tx, stream_rx) = tokio::sync::mpsc::channel(32);

        // Spawn task to forward broadcast messages to gRPC stream
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(notification) => {
                        let proto_notification = ConfigUpdateNotification {
                            config_version: notification.version,
                            timestamp_ms: notification.timestamp_ms,
                        };

                        if tx.send(Ok(proto_notification)).await.is_err() {
                            // Client disconnected
                            info!("Config update stream client disconnected");
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("Config update stream lagged, skipped {} messages", skipped);
                        // Continue receiving
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // Broadcaster closed
                        info!("Config broadcaster closed");
                        break;
                    }
                }
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(stream_rx);
        Ok(Response::new(stream))
    }
}

fn proto_from_metadata(metadata: CapabilityMetadataRuntime) -> super::proto::Capability {
    let kind = match metadata.kind {
        CapabilityKindConfig::ShellScript => super::proto::CapabilityKind::ShellScript as i32,
        CapabilityKindConfig::ShellInteractive => {
            super::proto::CapabilityKind::ShellInteractive as i32
        }
        CapabilityKindConfig::FileTransfer => super::proto::CapabilityKind::FileTransfer as i32,
    };

    let session_mode = match metadata.session_mode {
        CapabilitySessionMode::OneShot => super::proto::SessionMode::OneShot as i32,
        CapabilitySessionMode::Realtime => super::proto::SessionMode::Realtime as i32,
        CapabilitySessionMode::Upload => super::proto::SessionMode::Upload as i32,
        CapabilitySessionMode::Download => super::proto::SessionMode::Download as i32,
    };

    let parameters = metadata
        .parameters
        .iter()
        .map(proto_from_parameter)
        .collect();

    super::proto::Capability {
        id: metadata.id,
        name: metadata.name,
        description: metadata.description.unwrap_or_default(),
        tags: metadata.tags,
        kind,
        session_mode,
        parameters,
        requires_confirmation: Some(metadata.requires_confirmation),
        privileged: Some(metadata.privileged),
        version: None,
    }
}

fn proto_from_parameter(param: &CapabilityParameterRuntime) -> super::proto::CapabilityParameter {
    let param_type = match param.param_type.as_str() {
        "slider" => super::proto::CapabilityParameterType::Slider as i32,
        "text" => super::proto::CapabilityParameterType::Text as i32,
        "toggle" => super::proto::CapabilityParameterType::Toggle as i32,
        "dropdown" => super::proto::CapabilityParameterType::Dropdown as i32,
        _ => super::proto::CapabilityParameterType::Unspecified as i32,
    };

    super::proto::CapabilityParameter {
        name: param.name.clone(),
        r#type: param_type,
        description: param.description.clone().unwrap_or_default(),
        min: param.min,
        max: param.max,
        default_value: param.default_value.clone(),
        options: param.options.clone(),
        validation: param.validation.clone(),
        label_on: param.label_on.clone(),
        label_off: param.label_off.clone(),
        default_value_command: param.default_value_command.clone(),
        default_value_pattern: param.default_value_pattern.clone(),
    }
}

async fn forward_session_events(
    session_id: SessionId,
    capability_id: String,
    session_mode: CapabilitySessionMode,
    mut receiver: tokio::sync::broadcast::Receiver<SessionBroadcastEvent>,
    tx: tokio::sync::mpsc::Sender<Result<SessionServerMessage, Status>>,
) {
    let session_id_str = session_id.to_string();

    loop {
        match receiver.recv().await {
            Ok(event) => {
                let message = session_event_to_proto(
                    &session_id_str,
                    &capability_id,
                    session_mode,
                    event.event,
                    event.buffered_output.as_ref(),
                    event.resume_token.as_deref(),
                );
                if tx.send(Ok(message)).await.is_err() {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                warn!(
                    session_id = %session_id_str,
                    skipped,
                    "Session event consumer lagged behind broadcast buffer"
                );
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}

async fn forward_client_events(
    session_manager: crate::sessions::manager::SessionManager,
    session_id: SessionId,
    mut stream: tonic::Streaming<SessionClientMessage>,
    sender: tokio::sync::mpsc::Sender<SessionClientEvent>,
) {
    let session_id_str = session_id.to_string();

    loop {
        match stream.message().await {
            Ok(Some(message)) => {
                if !message.session_id.is_empty() && message.session_id != session_id_str {
                    warn!(
                        "Received session message for mismatched session_id={} (expected {})",
                        message.session_id, session_id_str
                    );
                    continue;
                }

                if let Some(event) = client_message_to_event(message) {
                    let forward = match &event {
                        SessionClientEvent::Heartbeat { .. } => {
                            let _ = session_manager.touch_heartbeat(&session_id);
                            true
                        }
                        SessionClientEvent::Resume { .. } => {
                            warn!(
                                "Received resume request on active attachment for session {}",
                                session_id_str
                            );
                            false
                        }
                        _ => true,
                    };

                    if forward && sender.send(event).await.is_err() {
                        break;
                    }
                }
            }
            Ok(None) => {
                break;
            }
            Err(status) => {
                warn!("Error reading client session stream: {}", status);
                break;
            }
        }
    }

    let _ = session_manager.detach(&session_id);
}

fn client_message_to_event(message: SessionClientMessage) -> Option<SessionClientEvent> {
    match message.payload {
        Some(session_client_message::Payload::Input(input)) => Some(SessionClientEvent::Input {
            data: Bytes::from(input.data),
            binary: input.binary.unwrap_or_default(),
        }),
        Some(session_client_message::Payload::Resize(resize)) => Some(SessionClientEvent::Resize {
            cols: resize.cols,
            rows: resize.rows,
        }),
        Some(session_client_message::Payload::Heartbeat(heartbeat)) => {
            Some(SessionClientEvent::Heartbeat {
                timestamp_ms: heartbeat.timestamp_ms,
            })
        }
        Some(session_client_message::Payload::Close(close)) => Some(SessionClientEvent::Close {
            reason: close.reason,
        }),
        Some(session_client_message::Payload::Resume(resume)) => Some(SessionClientEvent::Resume {
            resume_token: resume.resume_token,
            last_stdout_sequence: resume.last_output_sequence,
            last_stderr_sequence: resume.last_error_sequence,
        }),
        None => None,
        Some(session_client_message::Payload::Open(_)) => None,
    }
}

fn session_event_to_proto(
    session_id: &str,
    capability_id: &str,
    session_mode: CapabilitySessionMode,
    event: SessionServerEvent,
    buffered_output: Option<&crate::sessions::BufferedOutput>,
    resume_token: Option<&str>,
) -> SessionServerMessage {
    let session_mode_proto = match session_mode {
        CapabilitySessionMode::OneShot => super::proto::SessionMode::OneShot as i32,
        CapabilitySessionMode::Realtime => super::proto::SessionMode::Realtime as i32,
        CapabilitySessionMode::Upload => super::proto::SessionMode::Upload as i32,
        CapabilitySessionMode::Download => super::proto::SessionMode::Download as i32,
    };

    let payload = match event {
        SessionServerEvent::Ready { message } => {
            session_server_message::Payload::Ready(super::proto::SessionReady {
                capability_id: capability_id.to_string(),
                session_mode: session_mode_proto,
                message,
                resume_token: resume_token.unwrap_or_default().to_string(),
            })
        }
        SessionServerEvent::Output {
            data,
            stderr,
            binary,
            timestamp_ms,
        } => session_server_message::Payload::Output(super::proto::SessionOutput {
            data: data.to_vec(),
            stderr: Some(stderr),
            binary: Some(binary),
            timestamp_ms,
            sequence: buffered_output.map(|frame| frame.sequence).unwrap_or(0),
        }),
        SessionServerEvent::Exit {
            exit_code,
            timed_out,
            message,
        } => session_server_message::Payload::Exit(super::proto::SessionExit {
            exit_code,
            timed_out: Some(timed_out),
            message,
        }),
        SessionServerEvent::Error { message, code } => {
            session_server_message::Payload::Error(super::proto::SessionError { message, code })
        }
        SessionServerEvent::HeartbeatAck {
            timestamp_ms,
            latency_hint_ms,
        } => session_server_message::Payload::Heartbeat(super::proto::SessionHeartbeatAck {
            timestamp_ms,
            latency_hint_ms,
        }),
        SessionServerEvent::Closed { reason } => {
            session_server_message::Payload::Closed(super::proto::SessionClosed { reason })
        }
    };

    SessionServerMessage {
        session_id: session_id.to_string(),
        payload: Some(payload),
    }
}

/// Start the gRPC server with TLS
pub async fn start_server(
    addr: SocketAddr,
    service: RemoteControlService,
    server_cert: Arc<ServerCertificate>,
) -> Result<()> {
    info!("Starting gRPC server with TLS on {}", addr);

    let tls_config = build_permissive_server_config(&server_cert)
        .context("Failed to build TLS server configuration")?;
    let tls_acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let std_listener =
        bind_tcp_listener(addr).context("Failed to bind TCP listener for gRPC server")?;
    let listener =
        TcpListener::from_std(std_listener).context("Failed to create async TCP listener")?;
    let listener = Arc::new(listener);
    let acceptor = Arc::new(tls_acceptor);

    let incoming: Pin<
        Box<
            dyn Stream<
                    Item = Result<
                        tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
                        std::io::Error,
                    >,
                > + Send,
        >,
    > = {
        let listener = listener.clone();
        let acceptor = acceptor.clone();
        Box::pin(try_stream! {
            loop {
                let (socket, _) = listener.accept().await?;
                match acceptor.accept(socket).await {
                    Ok(stream) => yield stream,
                    Err(err) => {
                        warn!("TLS handshake failed: {}", err);
                    }
                }
            }
        })
    };

    Server::builder()
        .add_service(RemoteControlServer::new(service))
        .serve_with_incoming(incoming)
        .await
        .context("Failed to start gRPC server")?;

    Ok(())
}

fn bind_tcp_listener(addr: SocketAddr) -> Result<std::net::TcpListener> {
    let domain = match addr {
        SocketAddr::V4(_) => Domain::IPV4,
        SocketAddr::V6(_) => Domain::IPV6,
    };

    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))
        .context("Failed to create TCP socket")?;
    socket
        .set_reuse_address(true)
        .context("Failed to enable SO_REUSEADDR")?;

    if matches!(addr, SocketAddr::V6(_)) {
        // Allow the IPv6 listener to accept IPv4 connections as well (dual-stack)
        socket
            .set_only_v6(false)
            .context("Failed to configure dual-stack IPv6 listener")?;
    }

    socket
        .bind(&addr.into())
        .context("Failed to bind TCP socket to address")?;
    socket
        .listen(1024)
        .context("Failed to listen on TCP socket")?;
    socket
        .set_nonblocking(true)
        .context("Failed to set TCP socket to non-blocking mode")?;

    Ok(socket.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigBroadcaster;
    use crate::config::parser::{
        CapabilityAclConfig, CapabilityConfig, CapabilityDefinition, CapabilitySessionMode, Config,
        EnrollmentConfig, NetworkConfig, RelayConfig, SecurityConfig, ServerConfig,
        ShellScriptDefinition,
    };
    use crate::grpc::proto::ListCapabilitiesRequest;
    use crate::grpc::proto::remote_control_client::RemoteControlClient;
    use crate::notifications::{NotificationManager, NotificationProvider};
    use crate::security::certificates::ClientCertificate;
    use crate::security::enrollment::EnrollmentTokenManager;
    use crate::security::pairing::PairingRequestManager;
    use crate::storage::clients::ClientStore;
    use pem::Pem;
    use rcgen::generate_simple_self_signed;
    use std::io;
    use std::sync::atomic::AtomicU64;
    use std::sync::{Arc, Mutex, RwLock};
    use tempfile::TempDir;
    use tokio::sync::oneshot;
    use tonic::Request;
    use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};

    struct TestNotificationProvider;

    impl NotificationProvider for TestNotificationProvider {
        fn show_pairing_notification(
            &self,
            _device_name: &str,
            _verification_code: &str,
            _request_id: &str,
        ) -> anyhow::Result<bool> {
            Ok(false)
        }

        fn is_available(&self) -> bool {
            true
        }
    }

    struct TestHarness {
        addr: SocketAddr,
        shutdown: Option<oneshot::Sender<()>>,
        handle: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
        client_store: Arc<Mutex<ClientStore>>,
        server_cert_pem: String,
        _temp_dir: TempDir,
    }

    impl TestHarness {
        async fn new() -> io::Result<Self> {
            let temp_dir = TempDir::new().expect("create temp dir");
            let clients_dir = temp_dir.path().join("clients");
            let client_store = Arc::new(Mutex::new(
                ClientStore::new(clients_dir).expect("create client store"),
            ));

            let capability = CapabilityConfig {
                id: "echo".to_string(),
                name: "Echo".to_string(),
                description: Some("test".to_string()),
                tags: Vec::new(),
                requires_confirmation: false,
                privileged: false,
                parameters: Vec::new(),
                acl: CapabilityAclConfig::default(),
                definition: CapabilityDefinition::ShellScript(ShellScriptDefinition {
                    command: "echo test".to_string(),
                    timeout_seconds: 5,
                    env: std::collections::HashMap::new(),
                    show_output: true,
                    session_mode: CapabilitySessionMode::OneShot,
                }),
            };

            let config = Config {
                server: ServerConfig {
                    port: 0,
                    bind_address: "127.0.0.1".to_string(),
                    mdns_service_name: "test".to_string(),
                    mdns_instance_name: None,
                },
                security: SecurityConfig {
                    cert_path: None,
                    key_path: None,
                    authorized_clients_dir: None,
                    enrollment_token_ttl: 300,
                    require_client_cert: true,
                    enrollment: EnrollmentConfig::default(),
                },
                network: NetworkConfig::default(),
                relay: RelayConfig::default(),
                capabilities: vec![capability],
            };

            let config_arc = Arc::new(RwLock::new(config));
            let server_cert =
                Arc::new(ServerCertificate::generate().expect("generate server cert"));
            let server_id = Uuid::new_v4();

            let enrollment_manager = EnrollmentTokenManager::new(300);
            let pairing_manager = PairingRequestManager::new(60);
            let notification_manager =
                NotificationManager::with_provider(Box::new(TestNotificationProvider));
            let config_version = Arc::new(AtomicU64::new(1));
            let config_broadcaster = Arc::new(ConfigBroadcaster::new(16));
            let last_config_update = Arc::new(AtomicU64::new(0));

            let service = RemoteControlService::new(
                config_arc,
                server_cert.clone(),
                server_id,
                client_store.clone(),
                enrollment_manager,
                pairing_manager,
                notification_manager,
                config_version,
                config_broadcaster,
                last_config_update,
                None,
            );

            let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
            let addr = listener.local_addr()?;

            let tls_config =
                build_permissive_server_config(&server_cert).expect("build permissive tls config");
            let tls_acceptor = TlsAcceptor::from(Arc::new(tls_config));
            let listener = Arc::new(listener);
            let acceptor = Arc::new(tls_acceptor);

            let incoming: Pin<
                Box<
                    dyn Stream<
                            Item = Result<
                                tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
                                std::io::Error,
                            >,
                        > + Send,
                >,
            > = {
                let listener = listener.clone();
                let acceptor = acceptor.clone();
                Box::pin(try_stream! {
                    loop {
                        let (socket, _) = listener.accept().await?;
                        match acceptor.accept(socket).await {
                            Ok(stream) => yield stream,
                            Err(err) => {
                                warn!("TLS handshake failed during test: {}", err);
                            }
                        }
                    }
                })
            };

            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            let handle = tokio::spawn(async move {
                Server::builder()
                    .add_service(RemoteControlServer::new(service))
                    .serve_with_incoming_shutdown(incoming, async move {
                        let _ = shutdown_rx.await;
                    })
                    .await
            });

            let server_cert_pem =
                pem::encode(&Pem::new("CERTIFICATE", server_cert.cert_der.clone()));

            Ok(Self {
                addr,
                shutdown: Some(shutdown_tx),
                handle,
                client_store,
                server_cert_pem,
                _temp_dir: temp_dir,
            })
        }

        async fn shutdown(mut self) {
            if let Some(tx) = self.shutdown.take() {
                let _ = tx.send(());
            }
            match self.handle.await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => panic!("server error: {err}"),
                Err(join_err) => panic!("server task join failed: {join_err}"),
            }
        }
    }

    async fn make_client(
        harness: &TestHarness,
        identity: Option<Identity>,
    ) -> RemoteControlClient<tonic::transport::Channel> {
        let endpoint = Endpoint::from_shared(format!("https://localhost:{}", harness.addr.port()))
            .expect("create endpoint");
        let ca_cert = Certificate::from_pem(harness.server_cert_pem.clone());
        let mut tls = ClientTlsConfig::new()
            .ca_certificate(ca_cert)
            .domain_name("localhost");

        if let Some(identity) = identity {
            tls = tls.identity(identity);
        }

        let channel = endpoint.tls_config(tls).unwrap().connect().await.unwrap();
        RemoteControlClient::new(channel)
    }

    fn generate_client_identity(common_name: &str) -> (Identity, Vec<u8>) {
        let certified = generate_simple_self_signed(vec![common_name.to_string()])
            .expect("generate client certificate");
        let cert_der = certified.cert.der().to_vec();
        let cert_pem = certified.cert.pem().into_bytes();
        let key_pem = certified.signing_key.serialize_pem().into_bytes();
        let identity = Identity::from_pem(cert_pem, key_pem);

        (identity, cert_der)
    }

    #[tokio::test]
    async fn unauthenticated_client_rejected() {
        let harness = match TestHarness::new().await {
            Ok(h) => h,
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return,
            Err(e) => panic!("failed to initialize test harness: {e}"),
        };
        let mut client = make_client(&harness, None).await;

        let status = client
            .list_capabilities(Request::new(ListCapabilitiesRequest {}))
            .await
            .expect_err("unauthenticated client should be rejected");

        assert_eq!(status.code(), tonic::Code::Unauthenticated);

        harness.shutdown().await;
    }

    #[tokio::test]
    async fn unenrolled_client_rejected() {
        let harness = match TestHarness::new().await {
            Ok(h) => h,
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return,
            Err(e) => panic!("failed to initialize test harness: {e}"),
        };
        let (identity, _cert_der) = generate_client_identity("unenrolled");
        let mut client = make_client(&harness, Some(identity)).await;

        let status = client
            .list_capabilities(Request::new(ListCapabilitiesRequest {}))
            .await
            .expect_err("unenrolled client should be rejected");

        assert_eq!(status.code(), tonic::Code::PermissionDenied);

        harness.shutdown().await;
    }

    #[tokio::test]
    async fn enrolled_client_can_access_commands() {
        let harness = match TestHarness::new().await {
            Ok(h) => h,
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return,
            Err(e) => panic!("failed to initialize test harness: {e}"),
        };
        let (identity, cert_der) = generate_client_identity("enrolled");

        {
            let mut store = harness.client_store.lock().unwrap();
            let client_cert = ClientCertificate::from_der(cert_der.clone());
            store
                .add_client(&client_cert, "enrolled".to_string(), None)
                .expect("store client cert");
        }

        let mut client = make_client(&harness, Some(identity)).await;
        let response = client
            .list_capabilities(Request::new(ListCapabilitiesRequest {}))
            .await
            .expect("enrolled client should succeed")
            .into_inner();

        assert_eq!(response.capabilities.len(), 1);

        harness.shutdown().await;
    }
}
