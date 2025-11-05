pub mod certificates;
pub mod commands;
pub mod config;
pub mod discovery;
pub mod enrollment;
pub mod grpc_client;
mod relay;
pub mod storage;

pub mod proto {
    tonic::include_proto!("handcontrol.v1");
}

pub use certificates::CertificatePaths;
pub use commands::{
    execute_command, list_commands, open_capability_session, validate_parameters,
    CapabilitySession, CapabilitySessionEvent, CapabilitySessionSender, CommandKind, CommandList,
    CommandParameter, CommandParameterType, CommandSessionMode, CommandStreamEvent, CommandSummary,
};
pub use config::{
    CliConfig, ClientConfig, ConnectionConfig, DeviceConfig, DiscoveryConfig, NetworkConfig,
    RelayBehaviorConfig, TuiConfig,
};
pub use discovery::{discover_servers, DiscoveredServer};
pub use enrollment::{
    enroll_via_approval, enroll_via_qr, ApprovalEnrollmentInput, ApprovalEnrollmentOutcome,
    QrEnrollmentInput, QrEnrollmentOutcome,
};
pub use grpc_client::{
    connect_registered, connect_unauthenticated, connect_unverified, fetch_server_info,
    load_pinned_fingerprint, persist_fingerprint, HandControlClient, ServerInfoData,
};
pub use storage::{ServerRegistry, ServerRegistryEntry};
