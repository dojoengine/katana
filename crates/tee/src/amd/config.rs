//! Prover configuration and sources.

/// Configuration for the AMD attestation prover.
#[derive(Debug, Clone, Default)]
pub struct ProverConfig {
    /// Private key for SP1 network proving (optional for mock mode)
    pub private_key: Option<String>,
    /// SP1 RPC URL (optional, uses default if not specified)
    pub rpc_url: Option<String>,
    /// Skip time validity check (useful for testing with old attestations)
    pub skip_time_validity_check: bool,
}

impl ProverConfig {
    /// Create a new prover config with explicit values.
    pub fn new(
        private_key: Option<String>,
        rpc_url: Option<String>,
        skip_time_validity_check: bool,
    ) -> Self {
        Self { private_key, rpc_url, skip_time_validity_check }
    }

    /// Create config from environment variables.
    ///
    /// Reads:
    /// - `NETWORK_PRIVATE_KEY` - Private key for SP1 network proving (preferred)
    /// - `SP1_PRIVATE_KEY` - Private key for network proving (fallback)
    /// - `SP1_RPC_URL` - RPC URL for SP1 network
    /// - `SKIP_TIME_VALIDITY_CHECK` - Skip time validity (true/false)
    pub fn from_env() -> Self {
        Self {
            private_key: std::env::var("NETWORK_PRIVATE_KEY")
                .ok()
                .or_else(|| std::env::var("SP1_PRIVATE_KEY").ok()),
            rpc_url: std::env::var("SP1_RPC_URL").ok(),
            skip_time_validity_check: std::env::var("SKIP_TIME_VALIDITY_CHECK")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(false),
        }
    }

    /// Check if network proving is configured.
    pub fn has_network_key(&self) -> bool {
        self.private_key.is_some()
    }
}
