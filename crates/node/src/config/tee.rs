use katana_tee::TeeProviderType;

/// TEE configuration options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeeConfig {
    /// The type of TEE provider to use for attestation.
    pub provider_type: TeeProviderType,
}
