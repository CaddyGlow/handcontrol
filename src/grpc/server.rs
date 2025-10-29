use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use tracing::{info, warn};
use uuid::Uuid;

use super::proto::remote_control_server::{RemoteControl, RemoteControlServer};
use super::proto::{
    CheckPairingStatusRequest, CheckPairingStatusResponse, EnrollRequest, EnrollResponse,
    ExecuteCommandRequest, ExecuteCommandResponse, ListCommandsRequest, ListCommandsResponse,
    RequestPairingRequest, RequestPairingResponse, ServerInfoRequest, ServerInfoResponse,
};

use crate::config::Config;
use crate::notifications::NotificationManager;
use crate::security::certificates::{ClientCertificate, ServerCertificate};
use crate::security::enrollment::EnrollmentTokenManager;
use crate::security::pairing::{PairingRequestManager, PairingRequestStatus};
use crate::security::verification::generate_verification_code;
use crate::storage::clients::ClientStore;

/// gRPC service implementation
pub struct RemoteControlService {
    config: Arc<Config>,
    server_cert: Arc<ServerCertificate>,
    server_id: Uuid,
    client_store: Arc<Mutex<ClientStore>>,
    enrollment_manager: EnrollmentTokenManager,
    pairing_manager: PairingRequestManager,
    notification_manager: NotificationManager,
}

impl RemoteControlService {
    pub fn new(
        config: Arc<Config>,
        server_cert: Arc<ServerCertificate>,
        server_id: Uuid,
        client_store: Arc<Mutex<ClientStore>>,
        enrollment_manager: EnrollmentTokenManager,
        pairing_manager: PairingRequestManager,
        notification_manager: NotificationManager,
    ) -> Self {
        Self {
            config,
            server_cert,
            server_id,
            client_store,
            enrollment_manager,
            pairing_manager,
            notification_manager,
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

        info!(
            "Enrollment request from device: {}",
            req.device_name
        );

        // Check if QR code enrollment is enabled
        if !self.config.security.enrollment.qr_code_enabled {
            warn!("QR code enrollment is disabled");
            return Ok(Response::new(EnrollResponse {
                success: false,
                client_id: String::new(),
                error_message: "QR code enrollment is disabled on this server".to_string(),
            }));
        }

        // Validate enrollment token
        if let Err(e) = self.enrollment_manager.validate_and_consume(&req.enrollment_token) {
            warn!(
                "Enrollment failed for device {}: {}",
                req.device_name, e
            );
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
            return Err(Status::invalid_argument(
                "Client certificate is required",
            ));
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

    async fn request_pairing(
        &self,
        request: Request<RequestPairingRequest>,
    ) -> Result<Response<RequestPairingResponse>, Status> {
        let req = request.into_inner();

        info!(
            "Pairing request from device: {}",
            req.device_name
        );

        // Check if approval enrollment is enabled
        if !self.config.security.enrollment.approval_enabled {
            warn!("Approval mode enrollment is disabled");
            return Ok(Response::new(RequestPairingResponse {
                pending: false,
                pairing_request_id: String::new(),
                timeout_seconds: 0,
                verification_code: String::new(),
                server_cert_fingerprint: vec![],
                error_message: "Approval mode enrollment is disabled on this server".to_string(),
            }));
        }

        // Validate client certificate
        if req.client_certificate.is_empty() {
            warn!(
                "Pairing request failed for device {}: missing client certificate",
                req.device_name
            );
            return Err(Status::invalid_argument(
                "Client certificate is required",
            ));
        }

        // Generate verification code (server's computation)
        let server_verification_code = generate_verification_code(
            &req.client_certificate,
            &self.server_cert.cert_der,
            &self.server_id,
        );

        // MANDATORY: Verify client's code matches server's
        if req.verification_code != server_verification_code {
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
        if self.config.security.enrollment.approval_notification {
            match self.notification_manager.show_pairing_notification(
                &req.device_name,
                &server_verification_code,
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
            timeout_seconds: self.config.security.enrollment.approval_timeout_seconds as i32,
            verification_code: server_verification_code,
            server_cert_fingerprint: self.server_cert.fingerprint.to_vec(),
            error_message: String::new(),
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
                    pairing_request.client_id.as_ref().unwrap_or(&"unknown".to_string())
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

        // Convert config commands to protobuf format
        let commands: Vec<super::proto::Command> = self
            .config
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

        info!("Returning {} commands", commands.len());

        Ok(Response::new(ListCommandsResponse { commands }))
    }

    type ExecuteCommandStream =
        tokio_stream::wrappers::ReceiverStream<Result<ExecuteCommandResponse, Status>>;

    async fn execute_command(
        &self,
        request: Request<ExecuteCommandRequest>,
    ) -> Result<Response<Self::ExecuteCommandStream>, Status> {
        let req = request.into_inner();

        info!("ExecuteCommand RPC called: command_id={}", req.command_id);

        // Find command in config
        let command = self
            .config
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

                // Send output to stream (blocking send in callback)
                if let Err(e) = tx_clone.blocking_send(Ok(response)) {
                    // Client disconnected
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
}

/// Start the gRPC server
pub async fn start_server(
    addr: SocketAddr,
    service: RemoteControlService,
) -> Result<()> {
    info!("Starting gRPC server on {}", addr);

    Server::builder()
        .add_service(RemoteControlServer::new(service))
        .serve(addr)
        .await
        .context("Failed to start gRPC server")?;

    Ok(())
}
