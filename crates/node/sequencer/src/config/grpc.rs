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
            timeout: Some(Duration::from_secs(DEFAULT_GRPC_TIMEOUT_SECS)),
        }
    }
}
