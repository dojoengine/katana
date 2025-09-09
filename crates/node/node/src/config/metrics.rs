use std::net::{IpAddr, SocketAddr};

pub use katana_node_defaults::metrics::*;

/// Node metrics configurations.
#[derive(Debug, Copy, Clone)]
pub struct MetricsConfig {
    /// The address to bind the metrics server to.
    pub addr: IpAddr,
    /// The port to bind the metrics server to.
    pub port: u16,
}

impl MetricsConfig {
    /// Returns the [`SocketAddr`] for the metrics server.
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.addr, self.port)
    }
}
