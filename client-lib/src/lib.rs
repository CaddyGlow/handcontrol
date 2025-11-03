pub mod certificates;
pub mod config;
pub mod discovery;
pub mod storage;

pub mod proto {
    tonic::include_proto!("handcontrol.v1");
}

pub use certificates::CertificatePaths;
pub use config::{
    CliConfig, ClientConfig, ConnectionConfig, DeviceConfig, DiscoveryConfig, TuiConfig,
};
pub use discovery::{discover_servers, DiscoveredServer};
pub use storage::{ServerRegistry, ServerRegistryEntry};
