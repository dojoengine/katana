use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use katana_feeder_gateway::server::DEFAULT_GATEWAY_TIMEOUT;
use serde::{Deserialize, Serialize};

pub const DEFAULT_GATEWAY_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
pub const DEFAULT_GATEWAY_PORT: u16 = 5051;
pub const DEFAULT_GATEWAY_TIMEOUT_SECS: u64 = 20;

/// Configuration for the gateway server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayConfig {
    /// The IP address the gateway server will bind to.
    pub addr: IpAddr,
    /// The port number the gateway server will listen on.
    pub port: u16,
    /// The maximum duration to wait for a response from the gateway server.
    ///
    /// If `None`, requests will wait indefinitely. If `Some`, requests made to the gateway
    /// server will timeout after the specified duration has elapsed.
    pub timeout: Option<Duration>,
}

impl GatewayConfig {
    /// Returns the [`SocketAddr`] for the gateway server.
    pub const fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.addr, self.port)
    }
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            addr: DEFAULT_GATEWAY_ADDR,
            port: DEFAULT_GATEWAY_PORT,
            timeout: Some(DEFAULT_GATEWAY_TIMEOUT),
        }
    }
}
