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

    pub max_call_gas: Option<u64>,

    /// The maximum number of concurrent `estimate_fee` requests allowed.
    ///
    /// If `None`, defaults to [`DEFAULT_ESTIMATE_FEE_MAX_CONCURRENT_REQUESTS`].
    pub max_concurrent_estimate_fee_requests: Option<u32>,
}
