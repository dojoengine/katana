use katana_tee::AttesterKind;

/// TEE configuration options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeeConfig {
    /// The kind of attester to use for TEE attestation.
    pub attester: AttesterKind,
    /// The block number Katana forked from (resolved at startup).
    /// Included in TEE report_data for fork freshness verification.
    pub fork_block_number: Option<u64>,
}
