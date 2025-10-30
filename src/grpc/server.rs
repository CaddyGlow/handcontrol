use anyhow::{Context, Result};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use tracing::{info, warn};
use uuid::Uuid;

use super::proto::remote_control_server::{RemoteControl, RemoteControlServer};
use super::proto::{
    ApprovePairingRequest, ApprovePairingResponse, CheckPairingStatusRequest,
    CheckPairingStatusResponse, ConfigUpdateNotification, EnrollRequest, EnrollResponse,
    ExecuteCommandRequest, ExecuteCommandResponse, GenerateEnrollmentQrRequest,
    GenerateEnrollmentQrResponse, GetConfigVersionRequest, GetConfigVersionResponse,
    ListCommandsRequest, ListCommandsResponse, ListPendingPairingsRequest,
    ListPendingPairingsResponse, PendingPairingInfo, RequestPairingRequest,
    RequestPairingResponse, ServerInfoRequest, ServerInfoResponse, WatchConfigUpdatesRequest,
};

use crate::cli::approve::approve_pairing_request;
use crate::config::{Config, ConfigBroadcaster};
use crate::notifications::NotificationManager;
use crate::security::certificates::{ClientCertificate, ServerCertificate};
use crate::security::enrollment::EnrollmentTokenManager;
use crate::security::pairing::{PairingRequestManager, PairingRequestStatus};
use crate::security::verification::generate_verification_code;
// Network utilities (using qualified paths to avoid unused import warnings)
use crate::storage::clients::ClientStore;

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
        }
    }
}

#[tonic::async_trait]
impl RemoteControl for RemoteControlService {
    async fn enroll(
        &self,
        request: Request<EnrollRequest>,
    ) -> Result<Response<EnrollResponse>, Status> {
        let req = request.into_inner();

        info!("Enrollment request from device: {}", req.device_name);

        // Check if QR code enrollment is enabled
        let config = self.config.read().unwrap();
        if !config.security.enrollment.qr_code_enabled {
            warn!("QR code enrollment is disabled");
            return Ok(Response::new(EnrollResponse {
                success: false,
                client_id: String::new(),
                error_message: "QR code enrollment is disabled on this server".to_string(),
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
        let client_id = match store.add_client(&client_cert, req.device_name.clone()) {
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

        Ok(Response::new(EnrollResponse {
            success: true,
            client_id,
            error_message: String::new(),
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
                }));
            }
        };

        // Determine server IPs for clients to connect to
        let server_ips = if config.server.bind_address == "0.0.0.0"
            || config.server.bind_address == "::"
        {
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
        let payload = crate::utils::qr::EnrollmentQrPayload::new(
            server_ips.clone(),
            config.server.port,
            self.server_cert.fingerprint_display(),
            token.token.clone(),
            self.server_id,
        );

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
                }));
            }
        };

        info!("Generated enrollment token, expires in {} seconds", config.security.enrollment_token_ttl);

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
            relay_info: None,  // Future relay support
        }))
    }

    async fn request_pairing(
        &self,
        request: Request<RequestPairingRequest>,
    ) -> Result<Response<RequestPairingResponse>, Status> {
        let req = request.into_inner();

        info!("Pairing request from device: {}", req.device_name);

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
        rand::thread_rng().fill_bytes(&mut nonce);

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
        let req = request.into_inner();

        // Get pairing request
        let pairing_request = match self.pairing_manager.get_request(&req.pairing_request_id) {
            Some(request) => request,
            None => {
                warn!("Pairing request not found: {}", req.pairing_request_id);
                return Ok(Response::new(CheckPairingStatusResponse {
                    status: super::proto::PairingStatus::Unspecified as i32,
                    client_id: String::new(),
                    error_message: "Pairing request not found".to_string(),
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

        Ok(Response::new(CheckPairingStatusResponse {
            status,
            client_id,
            error_message,
        }))
    }

    async fn approve_pairing(
        &self,
        request: Request<ApprovePairingRequest>,
    ) -> Result<Response<ApprovePairingResponse>, Status> {
        let req = request.into_inner();
        info!(
            "ApprovePairing RPC called for request_id={}",
            req.pairing_request_id
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
        _request: Request<ListPendingPairingsRequest>,
    ) -> Result<Response<ListPendingPairingsResponse>, Status> {
        info!("ListPendingPairings RPC called");

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
                }
            })
            .collect();

        info!("Found {} pending pairing request(s)", requests.len());

        Ok(Response::new(ListPendingPairingsResponse { requests }))
    }

    async fn get_server_info(
        &self,
        _request: Request<ServerInfoRequest>,
    ) -> Result<Response<ServerInfoResponse>, Status> {
        info!("GetServerInfo RPC called");

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

    async fn list_commands(
        &self,
        _request: Request<ListCommandsRequest>,
    ) -> Result<Response<ListCommandsResponse>, Status> {
        info!("ListCommands RPC called");

        // Read config once and use it throughout
        let config = self.config.read().unwrap();
        let config_version = self.config_version.load(Ordering::SeqCst);

        // Convert config commands to protobuf format
        let commands: Vec<super::proto::Command> = config
            .command
            .iter()
            .map(|cmd| {
                let parameters = cmd
                    .parameters
                    .iter()
                    .map(|p| {
                        // Map parameter type string to protobuf enum
                        let param_type = match p.param_type.as_str() {
                            "slider" => super::proto::ParameterType::Slider as i32,
                            "text" => super::proto::ParameterType::Text as i32,
                            "toggle" => super::proto::ParameterType::Toggle as i32,
                            "dropdown" => super::proto::ParameterType::Dropdown as i32,
                            _ => super::proto::ParameterType::Unspecified as i32,
                        };

                        super::proto::Parameter {
                            name: p.name.clone(),
                            r#type: param_type,
                            description: p.description.clone().unwrap_or_default(),
                            min: p.min,
                            max: p.max,
                            default_value: p.default.clone(),
                            options: p.options.clone(),
                            validation: p.validation.clone(),
                            label_on: p.label_on.clone(),
                            label_off: p.label_off.clone(),
                        }
                    })
                    .collect();

                super::proto::Command {
                    id: cmd.id.clone(),
                    name: cmd.name.clone(),
                    description: cmd.description.clone().unwrap_or_default(),
                    icon: cmd.icon.clone().unwrap_or_default(),
                    tags: cmd.tags.clone(),
                    parameters,
                }
            })
            .collect();

        info!("Returning {} commands (config version {})", commands.len(), config_version);

        Ok(Response::new(ListCommandsResponse {
            commands,
            config_version,
        }))
    }

    type ExecuteCommandStream =
        tokio_stream::wrappers::ReceiverStream<Result<ExecuteCommandResponse, Status>>;

    async fn execute_command(
        &self,
        request: Request<ExecuteCommandRequest>,
    ) -> Result<Response<Self::ExecuteCommandStream>, Status> {
        let req = request.into_inner();

        info!("ExecuteCommand RPC called: command_id={}", req.command_id);

        // Read config and find command
        let config = self.config.read().unwrap();
        let command = config
            .command
            .iter()
            .find(|cmd| cmd.id == req.command_id)
            .ok_or_else(|| {
                warn!("Command not found: {}", req.command_id);
                Status::not_found(format!("Command '{}' not found", req.command_id))
            })?;

        // Validate parameters
        let validated_params = crate::commands::validate_parameters(command, &req.parameters)
            .map_err(|e| {
                warn!(
                    "Parameter validation failed for command {}: {}",
                    req.command_id, e
                );
                Status::invalid_argument(format!("Parameter validation failed: {}", e))
            })?;

        // Substitute parameters into shell command
        let shell_command =
            crate::commands::substitute_parameters(&command.shell, &validated_params).map_err(
                |e| {
                    warn!(
                        "Parameter substitution failed for command {}: {}",
                        req.command_id, e
                    );
                    Status::internal(format!("Parameter substitution failed: {}", e))
                },
            )?;

        // Create channel for streaming output
        let (tx, rx) = tokio::sync::mpsc::channel(128);

        // Clone command for async task
        let command = command.clone();
        let command_id = req.command_id.clone();

        // Spawn task to execute command
        tokio::spawn(async move {
            let tx_clone = tx.clone();

            let result = crate::commands::execute_command(&command, shell_command, move |output| {
                let response = match output {
                    crate::commands::CommandOutput::Stdout(text) => ExecuteCommandResponse {
                        response: Some(super::proto::execute_command_response::Response::Stdout(
                            text,
                        )),
                        timestamp_ms: Some(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as i64,
                        ),
                    },
                    crate::commands::CommandOutput::Stderr(text) => ExecuteCommandResponse {
                        response: Some(super::proto::execute_command_response::Response::Stderr(
                            text,
                        )),
                        timestamp_ms: Some(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as i64,
                        ),
                    },
                };

                // Send output to stream (non-blocking send in callback)
                if let Err(e) = tx_clone.try_send(Ok(response)) {
                    // Client disconnected or channel full
                    warn!("Failed to send output to stream: {}", e);
                }
            })
            .await;

            match result {
                Ok(exec_result) => {
                    info!(
                        "Command execution completed: id={}, exit_code={}, timed_out={}",
                        command_id, exec_result.exit_code, exec_result.timed_out
                    );

                    // Send final message with exit code
                    let final_response = ExecuteCommandResponse {
                        response: Some(super::proto::execute_command_response::Response::ExitCode(
                            exec_result.exit_code,
                        )),
                        timestamp_ms: Some(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as i64,
                        ),
                    };

                    let _ = tx.send(Ok(final_response)).await;
                }
                Err(e) => {
                    warn!("Command execution failed: id={}, error={}", command_id, e);

                    // Send error message
                    let error_response = ExecuteCommandResponse {
                        response: Some(super::proto::execute_command_response::Response::Error(
                            format!("Command execution failed: {}", e),
                        )),
                        timestamp_ms: Some(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as i64,
                        ),
                    };

                    let _ = tx.send(Ok(error_response)).await;
                }
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Response::new(stream))
    }

    async fn get_config_version(
        &self,
        _request: Request<GetConfigVersionRequest>,
    ) -> Result<Response<GetConfigVersionResponse>, Status> {
        info!("GetConfigVersion RPC called");

        let config_version = self.config_version.load(Ordering::SeqCst);
        let last_updated_ms = self.last_config_update.load(Ordering::SeqCst) as i64;

        info!("Returning config version {} (last updated: {}ms)", config_version, last_updated_ms);

        Ok(Response::new(GetConfigVersionResponse {
            config_version,
            last_updated_ms,
        }))
    }

    type WatchConfigUpdatesStream =
        tokio_stream::wrappers::ReceiverStream<Result<ConfigUpdateNotification, Status>>;

    async fn watch_config_updates(
        &self,
        _request: Request<WatchConfigUpdatesRequest>,
    ) -> Result<Response<Self::WatchConfigUpdatesStream>, Status> {
        info!("WatchConfigUpdates RPC called - starting config update stream");

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

/// Start the gRPC server with TLS
pub async fn start_server(
    addr: SocketAddr,
    service: RemoteControlService,
    cert_path: std::path::PathBuf,
    key_path: std::path::PathBuf,
) -> Result<()> {
    use tonic::transport::{Identity, ServerTlsConfig};

    info!("Starting gRPC server with TLS on {}", addr);

    // Load certificate and key from PEM files
    let cert_pem = std::fs::read(&cert_path)
        .with_context(|| format!("Failed to read certificate file: {}", cert_path.display()))?;
    let key_pem = std::fs::read(&key_path)
        .with_context(|| format!("Failed to read key file: {}", key_path.display()))?;

    // Create server identity from certificate and key
    let identity = Identity::from_pem(cert_pem, key_pem);

    // Configure TLS (without client certificate verification for now)
    // TODO: Implement custom client certificate verification (see IMPLEMENTATION_GAPS.md #18)
    let tls_config = ServerTlsConfig::new().identity(identity);

    let std_listener =
        bind_tcp_listener(addr).context("Failed to bind TCP listener for gRPC server")?;
    let listener =
        TcpListener::from_std(std_listener).context("Failed to create async TCP listener")?;
    let incoming = TcpListenerStream::new(listener);

    Server::builder()
        .tls_config(tls_config)
        .context("Failed to configure TLS")?
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
