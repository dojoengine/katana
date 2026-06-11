use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_primitives::block::BlockNumber;
use katana_rpc_types::tee::{BlockAttestation, EventProofResponse};

/// TEE API for generating hardware attestation quotes.
///
/// This API allows clients to request attestation quotes that
/// cryptographically bind the current blockchain state to a
/// hardware-backed measurement.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "tee"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "tee"))]
pub trait TeeApi {
    /// Generate a TEE attestation quote for the requested block state.
    ///
    /// The quote commits to the block's transition fields and to a versioned
    /// `katana_tee_config_hash` that the node precomputes from its chain spec
    /// at startup. Verifiers can recompute the same hash from on-chain config
    /// and reject attestations bound to a different environment.
    ///
    /// `prev_block_id` is optional and included in the response for
    /// transition-style flows; `None` is the genesis case.
    #[method(name = "generateQuote")]
    async fn generate_quote(
        &self,
        prev_block_id: Option<BlockNumber>,
        block_id: BlockNumber,
    ) -> RpcResult<BlockAttestation>;

    /// Get a Merkle inclusion proof for a specific event in a block.
    ///
    /// Returns a proof that event at `event_index` is included in the block's
    /// `events_commitment` (Merkle root). The `events_commitment` is bound to the
    /// TEE attestation via `report_data`, so this proof chain connects an individual
    /// event to the hardware attestation.
    #[method(name = "getEventProof")]
    async fn get_event_proof(
        &self,
        block_number: BlockNumber,
        event_index: u32,
    ) -> RpcResult<EventProofResponse>;
}
