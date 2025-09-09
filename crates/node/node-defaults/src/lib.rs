pub mod rpc {
    use std::net::{IpAddr, Ipv4Addr};

    /// Default RPC server address.
    pub const DEFAULT_RPC_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
    /// Default RPC server port.
    pub const DEFAULT_RPC_PORT: u16 = 5050;
    /// Default maximmum page size for the `starknet_getEvents` RPC method.
    pub const DEFAULT_RPC_MAX_EVENT_PAGE_SIZE: u64 = 1024;
    /// Default maximmum number of keys for the `starknet_getStorageProof` RPC method.
    pub const DEFAULT_RPC_MAX_PROOF_KEYS: u64 = 100;
    /// Default maximum gas for the `starknet_call` RPC method.
    pub const DEFAULT_RPC_MAX_CALL_GAS: u64 = 1_000_000_000;
}

pub mod metrics {
    use std::net::{IpAddr, Ipv4Addr};

    /// Metrics server default address.
    pub const DEFAULT_METRICS_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
    /// Metrics server default port.
    pub const DEFAULT_METRICS_PORT: u16 = 9100;
}

pub mod execution {
    pub const MAX_RECURSION_DEPTH: usize = 1000;
    pub const DEFAULT_INVOCATION_MAX_STEPS: u32 = 10_000_000;
    pub const DEFAULT_VALIDATION_MAX_STEPS: u32 = 1_000_000;
    pub const DEFAULT_ENABLE_NATIVE_COMPILATION: bool = false;
}
