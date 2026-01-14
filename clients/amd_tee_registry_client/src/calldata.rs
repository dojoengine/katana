//! Starknet Calldata Generation
//!
//! Convert SP1 Groth16 proofs into Starknet-compatible calldata using Garaga.
//!
//! # Overview
//!
//! This module transforms SP1 proofs into the format expected by the
//! Garaga SP1 Verifier on Starknet. The calldata is a list of BigUint values
//! that can be passed directly to Starknet contract calls.
//!
//! # Usage
//!
//! ```no_run
//! use amd_tee_registry_client::{OnchainProof, StarknetCalldata};
//!
//! # fn example(proof: OnchainProof) -> Result<(), Box<dyn std::error::Error>> {
//! // Generate Starknet calldata from proof
//! let calldata = StarknetCalldata::from_proof(&proof)?;
//!
//! // Get as hex strings for contract calls
//! let hex_values = calldata.to_hex_strings();
//!
//! // Or get raw BigUint values
//! let values = calldata.values();
//! # Ok(())
//! # }
//! ```

use crate::Error;
use garaga_rs::calldata::full_proof_with_hints::groth16::{
    get_groth16_calldata, get_sp1_vk, Groth16Proof,
};
use garaga_rs::definitions::CurveID;
use num_bigint::BigUint;
use tracing::info;

use crate::prover::OnchainProof;

/// Starknet calldata for on-chain proof verification.
///
/// Contains the formatted calldata that can be passed to the
/// Garaga SP1 Verifier contract on Starknet.
#[derive(Debug, Clone)]
pub struct StarknetCalldata {
    /// Raw calldata values as BigUint
    values: Vec<BigUint>,
}

impl StarknetCalldata {
    /// Create calldata from an SP1 Groth16 proof.
    ///
    /// This converts the SP1 proof format into Starknet-compatible calldata
    /// using the Garaga library.
    ///
    /// # Arguments
    /// * `proof` - The SP1 proof with public values
    ///
    /// # Returns
    /// * `StarknetCalldata` - Formatted calldata for Starknet
    ///
    /// # Errors
    /// * Returns error if proof bytes are empty (mock mode)
    /// * Returns error if calldata generation fails
    pub fn from_proof(proof: &OnchainProof) -> Result<Self, Error> {
        // Check for mock/empty proof
        if proof.onchain_proof.is_empty() {
            return Err(Error::Calldata(
                "Cannot generate calldata from empty proof (mock mode?)".to_string(),
            ));
        }

        info!("Generating Starknet calldata from SP1 proof");

        // Extract components from proof:
        // - vkey: The verification key (program ID)
        // - public_values: From raw_proof.journal
        // - proof_bytes: The onchain_proof bytes (includes 4-byte selector)

        // The verifier_id is the SP1 program vkey as bytes
        let vkey_bytes = proof.program_id.verifier_id.as_slice().to_vec();

        // The public values are in the raw_proof journal
        let public_values = proof.raw_proof.journal.to_vec();

        // The onchain_proof contains the groth16 proof with selector
        let proof_bytes = proof.onchain_proof.to_vec();

        info!(
            "  vkey: {} bytes, public_values: {} bytes, proof: {} bytes",
            vkey_bytes.len(),
            public_values.len(),
            proof_bytes.len()
        );

        // Create Garaga Groth16 proof structure
        // from_sp1 expects: (vkey, public_values, proof_with_selector)
        let groth16_proof = Groth16Proof::from_sp1(vkey_bytes, public_values, proof_bytes);

        // Get the universal SP1 Groth16 verification key from Garaga
        let sp1_vk = get_sp1_vk();

        // Generate Starknet calldata
        let values = get_groth16_calldata(&groth16_proof, &sp1_vk, CurveID::BN254)
            .map_err(|e| Error::Calldata(format!("Failed to generate calldata: {}", e)))?;

        info!("Generated {} calldata elements", values.len());

        Ok(Self { values })
    }

    /// Get the raw calldata values.
    pub fn values(&self) -> &[BigUint] {
        &self.values
    }

    /// Consume self and return the raw calldata values.
    pub fn into_values(self) -> Vec<BigUint> {
        self.values
    }

    /// Get the number of calldata elements.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Check if calldata is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Convert calldata to hex strings.
    ///
    /// Returns a vector of hex strings prefixed with "0x".
    pub fn to_hex_strings(&self) -> Vec<String> {
        self.values.iter().map(|v| format!("0x{:x}", v)).collect()
    }

    /// Convert calldata to a single newline-separated hex string.
    ///
    /// This format is compatible with Starknet Foundry's `read_txt` function.
    pub fn to_hex_file_content(&self) -> String {
        self.to_hex_strings().join("\n") + "\n"
    }

    /// Convert calldata to decimal strings.
    ///
    /// Returns a vector of decimal string representations.
    pub fn to_decimal_strings(&self) -> Vec<String> {
        self.values.iter().map(|v| v.to_string()).collect()
    }

    /// Save calldata to a file in hex format.
    ///
    /// This format is compatible with Starknet Foundry's `read_txt` function.
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<(), Error> {
        std::fs::write(path, self.to_hex_file_content())
            .map_err(|e| Error::Calldata(format!("Failed to write calldata file: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_string_conversion() {
        let calldata = StarknetCalldata {
            values: vec![
                BigUint::from(255u32),
                BigUint::from(4095u32),
                BigUint::from(0u32),
            ],
        };

        let hex_strings = calldata.to_hex_strings();
        assert_eq!(hex_strings[0], "0xff");
        assert_eq!(hex_strings[1], "0xfff");
        assert_eq!(hex_strings[2], "0x0");
    }

    #[test]
    fn test_hex_file_content() {
        let calldata = StarknetCalldata {
            values: vec![BigUint::from(1u32), BigUint::from(2u32)],
        };

        let content = calldata.to_hex_file_content();
        assert_eq!(content, "0x1\n0x2\n");
    }

    #[test]
    fn test_decimal_strings() {
        let calldata = StarknetCalldata {
            values: vec![BigUint::from(255u32), BigUint::from(1000u32)],
        };

        let decimal_strings = calldata.to_decimal_strings();
        assert_eq!(decimal_strings[0], "255");
        assert_eq!(decimal_strings[1], "1000");
    }
}
