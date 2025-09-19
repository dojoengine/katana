use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use katana_feeder_gateway::server::DEFAULT_FEEDER_GATEWAY_TIMEOUT;
use serde::{Deserialize, Serialize};

pub const DEFAULT_FEEDER_GATEWAY_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
pub const DEFAULT_FEEDER_GATEWAY_PORT: u16 = 5051;

/// Configuration for the feeder gateway server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeederGatewayConfig {
    /// The IP address the feeder gateway server will bind to.
    pub addr: IpAddr,
    /// The port number the feeder gateway server will listen on.
    pub port: u16,
    /// The maximum duration to wait for a response from the feeder gateway server.
    ///
    /// If `None`, requests will wait indefinitely. If `Some`, requests made to the feeder gateway
    /// server will timeout after the specified duration has elapsed.
    pub timeout: Option<Duration>,
}

impl FeederGatewayConfig {
    /// Returns the [`SocketAddr`] for the feeder gateway server.
    pub const fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.addr, self.port)
    }
}

impl Default for FeederGatewayConfig {
    fn default() -> Self {
        Self {
            addr: DEFAULT_FEEDER_GATEWAY_ADDR,
            port: DEFAULT_FEEDER_GATEWAY_PORT,
            timeout: Some(DEFAULT_FEEDER_GATEWAY_TIMEOUT),
        }
    }
}
