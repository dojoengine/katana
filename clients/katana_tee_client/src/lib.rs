//! Katana TEE Client
//!
//! This crate provides client functionality for interacting with Katana TEE
//! and generating SP1 proofs from attestation quotes.
//!
//! # Features
//!
//! - Fetch TEE attestation quotes from Katana RPC
//! - Generate SP1 Groth16 proofs from attestation quotes
//! - Verify proof structure
//!
//! # Architecture
//!
//! This crate is Katana-specific and uses `amd_tee_registry_client` for
//! the generic AMD TEE attestation proving functionality.
//!
//! # Example
//!
//! ```no_run
//! use katana_tee_client::{KatanaRpcClient, AmdAttestationProver, ProverConfig};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Fetch attestation from Katana RPC
//! let client = KatanaRpcClient::new("http://localhost:5050");
//! let attestation = client.generate_quote().await?;
//!
//! // Generate SP1 proof
//! let prover = AmdAttestationProver::new(ProverConfig::from_env());
//! let quote_bytes = attestation.quote_bytes()?;
//! let proof = prover.prove(&quote_bytes).await?;
//! println!("Proof generated: {:?}", proof.program_id.verifier_id);
//! # Ok(())
//! # }
//! ```

use serde::{Deserialize, Serialize};

pub mod error;
pub mod rpc;
pub mod starknet;

pub use amd_tee_registry_client::{
    AmdAttestationProver, OnchainProof, ProverConfig, Sp1NetworkBackend, StarknetCalldata,
    StarknetRegistryClient,
};
pub use error::Error;
pub use rpc::KatanaRpcClient;

/// Response from Katana TEE RPC `tee_generateQuote` endpoint.
///
/// This struct can be deserialized from both JSON files and RPC responses.
///
/// # Example JSON
/// ```json
/// {
///     "quote": "0x05000000...",
///     "stateRoot": "0x5da4151...",
///     "blockHash": "0x54d29b6...",
///     "blockNumber": 0
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeeQuoteResponse {
    /// The raw attestation quote bytes (hex-encoded with 0x prefix).
    /// This is the 1184-byte AMD SEV-SNP attestation report.
    pub quote: String,

    /// The state root at the attested block (hex-encoded Felt).
    pub state_root: String,

    /// The hash of the attested block (hex-encoded).
    pub block_hash: String,

    /// The number of the attested block.
    pub block_number: u64,

    /// Merkle root of all events in the attested block (hex-encoded Felt).
    /// Included in report_data: Poseidon(state_root, block_hash, fork_block, events_commitment).
    #[serde(default)]
    pub events_commitment: Option<String>,
}

impl TeeQuoteResponse {
    /// Load a TeeQuoteResponse from a JSON file.
    ///
    /// Accepts both raw TeeQuoteResponse JSON and JSON-RPC wrapped responses.
    pub fn from_json_file(path: &std::path::Path) -> Result<Self, Error> {
        let content =
            std::fs::read_to_string(path).map_err(amd_tee_registry_client::Error::from)?;
        Self::from_json_str(&content)
    }

    /// Load a TeeQuoteResponse from a JSON string.
    pub fn from_json_str(json: &str) -> Result<Self, Error> {
        // Try to parse as a JSON-RPC response first (has "result" field)
        #[derive(Deserialize)]
        struct Wrapper {
            result: TeeQuoteResponse,
        }
        if let Ok(wrapper) = serde_json::from_str::<Wrapper>(json) {
            return Ok(wrapper.result);
        }
        // Otherwise try to parse directly
        serde_json::from_str(json).map_err(|e| amd_tee_registry_client::Error::from(e).into())
    }

    /// Decode the quote hex string to raw bytes.
    ///
    /// Strips the `0x` prefix if present.
    pub fn quote_bytes(&self) -> Result<Vec<u8>, Error> {
        let hex_str = self.quote.strip_prefix("0x").unwrap_or(&self.quote);
        hex::decode(hex_str)
            .map_err(|e| amd_tee_registry_client::Error::HexDecode(e.to_string()).into())
    }
}
