use katana_executor::ExecutionFlags;

#[derive(Debug, Clone)]
pub struct StarknetApiConfig {
    /// The max chunk size that can be served from the `getEvents` method.
    ///
    /// If `None`, the maximum chunk size is bounded by [`u64::MAX`].
    pub max_event_page_size: Option<u64>,

    /// The max keys whose proofs can be requested for from the `getStorageProof` method.
    ///
    /// If `None`, the maximum keys size is bounded by [`u64::MAX`].
    pub max_proof_keys: Option<u64>,

    /// Maximum Sierra gas for contract calls.
    ///
    /// The maximum amount of execution resources allocated for a contract call via `starknet_call`
    /// method. If `None,` defaults to `1,000,000,000`.
    ///
    /// ## Implementation Details
    ///
    /// If the contract call execution is tracked using Cairo steps (eg., the class is using an old
    /// sierra compiler version), this Sierra gas value must be converted to Cairo steps. Check out
    /// the [`call`] module for more information.
    ///
    /// [`call`]: katana_executor::implementation::blockifier::call
    pub max_call_gas: Option<u64>,

    /// The maximum number of concurrent `estimate_fee` requests allowed.
    ///
    /// If `None`, defaults to [`DEFAULT_ESTIMATE_FEE_MAX_CONCURRENT_REQUESTS`].
    pub max_concurrent_estimate_fee_requests: Option<u32>,

    /// Simulation flags used for fee simulation and estimation. (ie starknet_estimateFee and
    /// starknet_simulateTransactions)
    pub simulation_flags: ExecutionFlags,

    #[cfg(feature = "cartridge")]
    pub paymaster: Option<PaymasterConfig>,
}

#[cfg(feature = "cartridge")]
#[derive(Debug, Clone)]
pub struct PaymasterConfig {
    /// The root URL for the Cartridge API.
    pub cartridge_api_url: url::Url,
}
