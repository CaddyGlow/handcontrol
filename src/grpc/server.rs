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
use crate::security::certificates::{ClientCertificate, ServerCertificate};
use crate::security::enrollment::EnrollmentTokenManager;
use crate::storage::clients::ClientStore;

/// gRPC service implementation
pub struct RemoteControlService {
    #[allow(dead_code)] // Will be used in Phase 6 (Commands)
    config: Arc<Config>,
    server_cert: Arc<ServerCertificate>,
    server_id: Uuid,
    client_store: Arc<Mutex<ClientStore>>,
    enrollment_manager: EnrollmentTokenManager,
}

impl RemoteControlService {
    pub fn new(
        config: Arc<Config>,
        server_cert: Arc<ServerCertificate>,
        server_id: Uuid,
        client_store: Arc<Mutex<ClientStore>>,
        enrollment_manager: EnrollmentTokenManager,
    ) -> Self {
        Self {
            config,
            server_cert,
            server_id,
            client_store,
            enrollment_manager,
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
        _request: Request<RequestPairingRequest>,
    ) -> Result<Response<RequestPairingResponse>, Status> {
        warn!("RequestPairing RPC not yet implemented");
        Err(Status::unimplemented("RequestPairing not yet implemented"))
    }

    async fn check_pairing_status(
        &self,
        _request: Request<CheckPairingStatusRequest>,
    ) -> Result<Response<CheckPairingStatusResponse>, Status> {
        warn!("CheckPairingStatus RPC not yet implemented");
        Err(Status::unimplemented(
            "CheckPairingStatus not yet implemented",
        ))
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
        warn!("ListCommands RPC not yet implemented");
        Err(Status::unimplemented("ListCommands not yet implemented"))
    }

    type ExecuteCommandStream =
        tokio_stream::wrappers::ReceiverStream<Result<ExecuteCommandResponse, Status>>;

    async fn execute_command(
        &self,
        _request: Request<ExecuteCommandRequest>,
    ) -> Result<Response<Self::ExecuteCommandStream>, Status> {
        warn!("ExecuteCommand RPC not yet implemented");
        Err(Status::unimplemented("ExecuteCommand not yet implemented"))
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
