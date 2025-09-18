use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const DEFAULT_FEEDER_GATEWAY_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
pub const DEFAULT_FEEDER_GATEWAY_PORT: u16 = 5051;

/// Configuration for the feeder gateway server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeederGatewayConfig {
    pub addr: IpAddr,
    pub port: u16,
    pub timeout: Option<Duration>,
    /// Whether to share the RPC server's port instead of running independently.
    pub share_rpc_port: bool,
}

impl FeederGatewayConfig {
    /// Returns the [`SocketAddr`] for the feeder gateway server.
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.addr, self.port)
    }
}

impl Default for FeederGatewayConfig {
    fn default() -> Self {
        Self {
            addr: DEFAULT_FEEDER_GATEWAY_ADDR,
            port: DEFAULT_FEEDER_GATEWAY_PORT,
            timeout: Some(Duration::from_secs(30)),
            share_rpc_port: false,
        }
    }
}
