//! TEE RPC API implementation.

use std::sync::Arc;

use jsonrpsee::core::{async_trait, RpcResult};
use katana_primitives::Felt;
use katana_provider::api::block::{BlockHashProvider, BlockNumberProvider, HeaderProvider};
use katana_provider::ProviderFactory;
use katana_rpc_api::error::tee::TeeApiError;
use katana_rpc_api::tee::{TeeApiServer, TeeQuoteResponse};
use katana_tee::TeeProvider;
use starknet_types_core::hash::{Poseidon, StarkHash};
use tracing::{debug, info};

/// TEE API implementation.
#[allow(missing_debug_implementations)]
pub struct TeeApi<PF>
where
    PF: ProviderFactory,
{
    /// Storage provider factory for accessing blockchain state.
    provider_factory: PF,
    /// TEE provider for generating attestation quotes.
    tee_provider: Arc<dyn TeeProvider>,
}

impl<PF> TeeApi<PF>
where
    PF: ProviderFactory,
{
    /// Create a new TEE API instance.
    pub fn new(provider_factory: PF, tee_provider: Arc<dyn TeeProvider>) -> Self {
        info!(
            target: "rpc::tee",
            provider_type = tee_provider.provider_type(),
            "TEE API initialized"
        );
        Self { provider_factory, tee_provider }
    }

    /// Compute the 64-byte report data for attestation.
    ///
    /// The report data is: Poseidon(state_root, block_hash) padded to 64 bytes.
    fn compute_report_data(&self, state_root: Felt, block_hash: Felt) -> [u8; 64] {
        // Compute Poseidon hash of state_root and block_hash
        let commitment = Poseidon::hash(&state_root, &block_hash);

        // Convert Felt to bytes (32 bytes) and pad to 64 bytes
        let commitment_bytes = commitment.to_bytes_be();

        let mut report_data = [0u8; 64];
        // Place the 32-byte hash in the first half
        report_data[..32].copy_from_slice(&commitment_bytes);
        // Second half remains zeros (or could include additional metadata)

        debug!(
            target: "rpc::tee",
            %state_root,
            %block_hash,
            %commitment,
            "Computed report data for attestation"
        );

        report_data
    }
}

#[async_trait]
impl<PF> TeeApiServer for TeeApi<PF>
where
    PF: ProviderFactory + Send + Sync + 'static,
    <PF as ProviderFactory>::Provider:
        BlockHashProvider + BlockNumberProvider + HeaderProvider + Send + Sync,
{
    async fn generate_quote(&self) -> RpcResult<TeeQuoteResponse> {
        debug!(target: "rpc::tee", "Generating TEE attestation quote");

        // Get the latest blockchain state
        let provider = self.provider_factory.provider();

        // Get latest block information
        let block_number =
            provider.latest_number().map_err(|e| TeeApiError::ProviderError(e.to_string()))?;

        let block_hash =
            provider.latest_hash().map_err(|e| TeeApiError::ProviderError(e.to_string()))?;

        // Get the header to retrieve state_root
        let header = provider
            .header_by_number(block_number)
            .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
            .ok_or_else(|| {
                TeeApiError::ProviderError(format!("Header not found for block {block_number}"))
            })?;

        let state_root = header.state_root;

        // Compute report data: Poseidon(state_root, block_hash)
        let report_data = self.compute_report_data(state_root, block_hash);

        // Generate the attestation quote
        let quote = self
            .tee_provider
            .generate_quote(&report_data)
            .map_err(|e| TeeApiError::QuoteGenerationFailed(e.to_string()))?;

        info!(
            target: "rpc::tee",
            block_number,
            %block_hash,
            quote_size = quote.len(),
            "Generated TEE attestation quote"
        );

        Ok(TeeQuoteResponse {
            quote: format!("0x{}", hex::encode(&quote)),
            state_root,
            block_hash,
            block_number,
        })
    }
}
