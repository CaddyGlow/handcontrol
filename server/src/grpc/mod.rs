pub mod enrollment;
pub mod middleware;
pub mod server;

// Include the generated protobuf code
pub mod proto {
    tonic::include_proto!("handcontrol.v1");
}
