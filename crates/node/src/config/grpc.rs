//! gRPC server configuration for the node.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

/// Default gRPC server listening address.
pub const DEFAULT_GRPC_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// Default gRPC server listening port.
pub const DEFAULT_GRPC_PORT: u16 = 5051;

/// Default gRPC request timeout in seconds.
pub const DEFAULT_GRPC_TIMEOUT_SECS: u64 = 30;

/// Configuration for the gRPC server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpcConfig {
    /// The IP address to bind the server to.
    pub addr: IpAddr,
    /// The port to listen on.
    pub port: u16,
    /// Maximum number of concurrent connections.
    pub max_connections: Option<u32>,
    /// Request timeout in seconds.
    pub timeout: Option<Duration>,
}

impl GrpcConfig {
    /// Returns the socket address for the server.
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.addr, self.port)
    }
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            addr: DEFAULT_GRPC_ADDR,
            port: DEFAULT_GRPC_PORT,
            max_connections: None,
            timeout: Some(Duration::from_secs(DEFAULT_GRPC_TIMEOUT_SECS)),
        }
    }
}

#[cfg(feature = "grpc")]
impl From<GrpcConfig> for katana_grpc::GrpcConfig {
    fn from(config: GrpcConfig) -> Self {
        let mut grpc_config = katana_grpc::GrpcConfig::new(config.addr, config.port);
        if let Some(max_connections) = config.max_connections {
            grpc_config = grpc_config.with_max_connections(max_connections);
        }
        if let Some(timeout) = config.timeout {
            grpc_config = grpc_config.with_timeout(timeout);
        }
        grpc_config
    }
}
