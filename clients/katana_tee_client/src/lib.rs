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
//! use katana_tee_client::{KatanaRpcClient, generate_sp1_proof};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Fetch attestation from Katana RPC
//! let client = KatanaRpcClient::new("http://localhost:5050");
//! let attestation = client.generate_quote().await?;
//!
//! // Generate SP1 proof (uses amd_tee_registry_client internally)
//! let proof = generate_sp1_proof(attestation).await?;
//! println!("Proof generated: {:?}", proof.program_id.verifier_id);
//! # Ok(())
//! # }
//! ```

use serde::{Deserialize, Serialize};

pub mod error;
pub mod prover;
pub mod rpc;
pub mod starknet;

pub use error::Error;
pub use prover::{generate_sp1_proof, verify_proof_structure, OnchainProof, ProverConfig};
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
}

impl TeeQuoteResponse {
    /// Load a TeeQuoteResponse from a JSON file.
    pub fn from_json_file(path: &std::path::Path) -> Result<Self, Error> {
        let file = std::fs::File::open(path).map_err(|e| Error::Io(e.to_string()))?;
        let wrapper: JsonRpcResponse = serde_json::from_reader(file)?;
        Ok(wrapper.result)
    }

    /// Load a TeeQuoteResponse from a JSON string.
    pub fn from_json_str(json: &str) -> Result<Self, Error> {
        // Try to parse as a JSON-RPC response first
        if let Ok(wrapper) = serde_json::from_str::<JsonRpcResponse>(json) {
            return Ok(wrapper.result);
        }
        // Otherwise try to parse directly
        Ok(serde_json::from_str(json)?)
    }

    /// Decode the quote hex string to raw bytes.
    ///
    /// Strips the `0x` prefix if present.
    pub fn quote_bytes(&self) -> Result<Vec<u8>, Error> {
        let hex_str = self.quote.strip_prefix("0x").unwrap_or(&self.quote);
        hex::decode(hex_str).map_err(|e| Error::HexDecode(e.to_string()))
    }
}

/// JSON-RPC response wrapper for Katana RPC responses.
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: u64,
    result: TeeQuoteResponse,
}
