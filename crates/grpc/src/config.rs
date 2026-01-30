//! gRPC server configuration.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

/// Default gRPC server listening address.
pub const DEFAULT_GRPC_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// Default gRPC server listening port.
pub const DEFAULT_GRPC_PORT: u16 = 5051;

/// Default gRPC request timeout in seconds.
pub const DEFAULT_GRPC_TIMEOUT_SECS: u64 = 30;

/// Configuration for the gRPC server.
#[derive(Debug, Clone)]
pub struct GrpcConfig {
    /// The IP address to bind the server to.
    pub addr: IpAddr,
    /// The port to listen on.
    pub port: u16,
    /// Maximum number of concurrent connections.
    pub max_connections: Option<u32>,
    /// Request timeout.
    pub timeout: Option<Duration>,
}

impl GrpcConfig {
    /// Creates a new gRPC configuration with the given address and port.
    pub fn new(addr: IpAddr, port: u16) -> Self {
        Self { addr, port, max_connections: None, timeout: None }
    }

    /// Sets the maximum number of concurrent connections.
    pub fn with_max_connections(mut self, max: u32) -> Self {
        self.max_connections = Some(max);
        self
    }

    /// Sets the request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

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
