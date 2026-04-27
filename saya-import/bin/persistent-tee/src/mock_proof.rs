//! Helpers for synthesizing fake `TEEInput.sp1_proof` payloads when running
//! `saya-tee --mock-prove`.
//!
//! In mock mode the prover does **not** call AMD KDS, validate any cert chain,
//! or submit anything to the SP1 prover network. Instead it computes the
//! Poseidon commitment that Piltover's `validate_input` would otherwise extract
//! from a real attestation report and packages it into a Cairo-Serde-serialized
//! [`amd_tee_registry::tee_types::VerifierJournal`] which the paired
//! `piltover_mock_amd_tee_registry` Cairo contract trivially round-trips.
//!
//! The serialized journal is stored verbatim in [`TeeProof::data`] and is
//! consumed unmodified by [`crate::settlement::build_tee_calldata`] when its
//! `mock_prove` branch is taken — no `OnchainProof::decode_json` /
//! `StarknetCalldata::from_proof` round-trip is performed.
//!
//! ## Wire format of `TEEInput.sp1_proof` in mock mode
//!
//! A Cairo `Span<felt252>` matching the Cairo Serde of:
//!
//! ```cairo
//! VerifierJournal {
//!     result: VerificationResult::Success,   // 1 felt (variant 0)
//!     timestamp: 0,                          // 1 felt (u64)
//!     processor_model: 0,                    // 1 felt (u8, Milan)
//!     raw_report: Span<u32> { len = 296, .. }, // 1 + 296 felts
//!     certs: Array<u256> { len = 0 },        // 1 felt
//!     cert_serials: Array<felt252> { len = 0 }, // 1 felt
//!     trusted_certs_prefix_len: 0,           // 1 felt (u8)
//!     storage_commitment: 0,                 // 1 felt (felt252)
//!     fork_block_number: 0,                  // 1 felt (u64)
//!     end_block_number: 0,                   // 1 felt (u64)
//! }
//! ```
//!
//! Within `raw_report`, only the 16 u32 words at the `report_data` offset
//! (u32 index 20) carry meaningful data; all other words are zero.
//!
//! ## `report_data` byte layout
//!
//! Piltover (`src/input/component.cairo`, `validate_input` for `TeeInput`)
//! computes:
//!
//! ```cairo
//! let raw_report = RawAttestationReport { raw: journal.raw_report };
//! let report_data = raw_report.report_data();          // u512
//! assert!(report_data.limb2 == 0 && report_data.limb3 == 0);
//! let expected_commitment = u256 {
//!     low:  u128_byte_reverse(report_data.limb1),
//!     high: u128_byte_reverse(report_data.limb0),
//! };
//! ```
//!
//! `get_u128_at` reads 4 consecutive u32 words and combines them
//! **little-endian**: `limb = w0 + w1·2^32 + w2·2^64 + w3·2^96`. Then
//! `u128_byte_reverse` swaps the byte order, converting LE → BE. Composing
//! these:
//!
//! - `expected_commitment.high == BE_u128(bytes 0..16 of report_data)`
//! - `expected_commitment.low  == BE_u128(bytes 16..32 of report_data)`
//!
//! Therefore `report_data` bytes 0..32 are exactly `commitment.to_bytes_be()`
//! (the 32-byte big-endian encoding of the felt commitment as a `u256`), and
//! bytes 32..64 must be zero.
//!
//! Packing those 32 BE bytes into 8 u32 words for `raw_report[20..28]`
//! requires reading each 4-byte chunk **little-endian** so that
//! `get_u128_at`'s LE recombination produces the correct limb. See
//! [`commitment_to_report_words`] for the implementation.
//!
//! Piltover then asserts `expected_commitment` equals
//! `Poseidon(prev_state_root, state_root, prev_block_hash, block_hash,
//!           prev_block_number, block_number, messages_commitment)`, which the
//! mock prover computes ahead of time and embeds via this layout.

use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Poseidon, StarkHash};

/// Number of u32 words in an AMD SEV-SNP attestation report (1184 bytes / 4).
pub const ATTESTATION_REPORT_WORDS: usize = 296;

/// u32 word offset of the `report_data` field within an attestation report
/// (byte offset 0x50 / 4).
pub const REPORT_DATA_WORD_OFFSET: usize = 20;

/// Computes the Poseidon commitment Piltover asserts against `report_data` for
/// the appchain (non-fork) TEE settlement path.
///
/// Mirrors `compute_report_data_appchain` in
/// `katana::crates::rpc::rpc-server::src::tee.rs`.
pub fn compute_appchain_commitment(
    prev_state_root: Felt,
    state_root: Felt,
    prev_block_hash: Felt,
    block_hash: Felt,
    prev_block_number: Felt,
    block_number: Felt,
    messages_commitment: Felt,
) -> Felt {
    Poseidon::hash_array(&[
        prev_state_root,
        state_root,
        prev_block_hash,
        block_hash,
        prev_block_number,
        block_number,
        messages_commitment,
    ])
}

/// Encodes a 256-bit commitment into the 8 u32 words at the `report_data`
/// offset such that Piltover's `expected_commitment` reconstruction yields
/// the original commitment.
///
/// Each 4-byte BE chunk of `commitment.to_bytes_be()` is interpreted as a
/// little-endian `u32`. See module docs for the derivation.
fn commitment_to_report_words(commitment: Felt) -> [Felt; 8] {
    let bytes = commitment.to_bytes_be();
    let mut words = [Felt::ZERO; 8];
    for i in 0..8 {
        let chunk = [
            bytes[i * 4],
            bytes[i * 4 + 1],
            bytes[i * 4 + 2],
            bytes[i * 4 + 3],
        ];
        let word = u32::from_le_bytes(chunk);
        words[i] = Felt::from(word);
    }
    words
}

/// Builds a 296-word `raw_report` whose `report_data` field encodes the given
/// commitment per the layout documented in [`commitment_to_report_words`]. All
/// other words are zero.
pub fn build_raw_report(commitment: Felt) -> Vec<Felt> {
    let mut raw_report = vec![Felt::ZERO; ATTESTATION_REPORT_WORDS];
    let words = commitment_to_report_words(commitment);
    raw_report[REPORT_DATA_WORD_OFFSET..REPORT_DATA_WORD_OFFSET + 8].copy_from_slice(&words);
    // Words [28..36) (limb2 + limb3 of report_data) remain zero, satisfying the
    // `assert!(report_data.limb2 == 0 && report_data.limb3 == 0)` check.
    raw_report
}

/// Cairo-Serde-serializes a stub `VerifierJournal` whose `raw_report` field
/// encodes the given Poseidon commitment in the position Piltover reads.
///
/// The output is a `Vec<Felt>` matching what
/// `Serde::<VerifierJournal>::deserialize` reconstructs in
/// `piltover_mock_amd_tee_registry::verify_sp1_proof`.
pub fn serialize_mock_journal(commitment: Felt) -> Vec<Felt> {
    let raw_report = build_raw_report(commitment);

    // 1 (result) + 1 (timestamp) + 1 (processor_model)
    // + 1 (raw_report len) + 296 (raw_report elements)
    // + 1 (certs len) + 1 (cert_serials len) + 1 (trusted_certs_prefix_len)
    // + 1 (storage_commitment) + 1 (fork_block_number) + 1 (end_block_number)
    // = 306 felts.
    let mut felts = Vec::with_capacity(306);

    // result: VerificationResult::Success → variant index 0
    felts.push(Felt::ZERO);
    // timestamp: u64
    felts.push(Felt::ZERO);
    // processor_model: u8 (Milan == 0)
    felts.push(Felt::ZERO);
    // raw_report: Span<u32> = [length, ...elements]
    felts.push(Felt::from(raw_report.len() as u64));
    felts.extend(raw_report);
    // certs: Array<u256> = [length, ...]
    felts.push(Felt::ZERO);
    // cert_serials: Array<felt252> = [length, ...]
    felts.push(Felt::ZERO);
    // trusted_certs_prefix_len: u8
    felts.push(Felt::ZERO);
    // storage_commitment: felt252
    felts.push(Felt::ZERO);
    // fork_block_number: u64
    felts.push(Felt::ZERO);
    // end_block_number: u64
    felts.push(Felt::ZERO);

    felts
}

/// Encodes a `Vec<Felt>` as raw big-endian bytes for storage in
/// [`crate::TeeProof::data`]. Each felt occupies exactly 32 bytes.
pub fn felts_to_bytes(felts: &[Felt]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(felts.len() * 32);
    for felt in felts {
        bytes.extend_from_slice(&felt.to_bytes_be());
    }
    bytes
}

/// Decodes a `Vec<Felt>` from raw big-endian bytes produced by
/// [`felts_to_bytes`].
///
/// Returns `None` if the byte slice length is not a multiple of 32.
pub fn bytes_to_felts(bytes: &[u8]) -> Option<Vec<Felt>> {
    if !bytes.len().is_multiple_of(32) {
        return None;
    }
    let mut felts = Vec::with_capacity(bytes.len() / 32);
    for chunk in bytes.chunks_exact(32) {
        let mut buf = [0u8; 32];
        buf.copy_from_slice(chunk);
        felts.push(Felt::from_bytes_be(&buf));
    }
    Some(felts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn felt_round_trip_via_bytes() {
        let felts = vec![
            Felt::ZERO,
            Felt::from(1u64),
            Felt::from(0xdeadbeef_u64),
            Felt::from_hex("0x123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap(),
        ];
        let bytes = felts_to_bytes(&felts);
        let decoded = bytes_to_felts(&bytes).unwrap();
        assert_eq!(decoded, felts);
    }

    #[test]
    fn raw_report_has_canonical_size() {
        let raw_report = build_raw_report(Felt::from(42u64));
        assert_eq!(raw_report.len(), ATTESTATION_REPORT_WORDS);
    }

    #[test]
    fn report_data_zero_outside_first_32_bytes() {
        let raw_report = build_raw_report(Felt::from(42u64));
        // Words [0..20) and [28..296) must be zero.
        for (i, word) in raw_report.iter().enumerate().take(20) {
            assert_eq!(*word, Felt::ZERO, "word {i} should be zero");
        }
        for (i, word) in raw_report
            .iter()
            .enumerate()
            .take(ATTESTATION_REPORT_WORDS)
            .skip(28)
        {
            assert_eq!(*word, Felt::ZERO, "word {i} should be zero");
        }
    }

    #[test]
    fn report_data_round_trips_commitment() {
        // Mirror Piltover's reconstruction:
        //   limb_i = sum(w_{4i+j} * 2^(32*j)) for j in 0..4   (little-endian u32 → u128)
        //   high   = u128_byte_reverse(limb0)
        //   low    = u128_byte_reverse(limb1)
        //   commitment = (high << 128) | low
        let commitment =
            Felt::from_hex("0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap();
        let raw_report = build_raw_report(commitment);

        let read_limb = |start: usize| -> u128 {
            let w0 = u128::from(
                raw_report[start].to_bytes_be()[31] as u32
                    | (raw_report[start].to_bytes_be()[30] as u32) << 8
                    | (raw_report[start].to_bytes_be()[29] as u32) << 16
                    | (raw_report[start].to_bytes_be()[28] as u32) << 24,
            );
            let w1 = u128::from(
                raw_report[start + 1].to_bytes_be()[31] as u32
                    | (raw_report[start + 1].to_bytes_be()[30] as u32) << 8
                    | (raw_report[start + 1].to_bytes_be()[29] as u32) << 16
                    | (raw_report[start + 1].to_bytes_be()[28] as u32) << 24,
            );
            let w2 = u128::from(
                raw_report[start + 2].to_bytes_be()[31] as u32
                    | (raw_report[start + 2].to_bytes_be()[30] as u32) << 8
                    | (raw_report[start + 2].to_bytes_be()[29] as u32) << 16
                    | (raw_report[start + 2].to_bytes_be()[28] as u32) << 24,
            );
            let w3 = u128::from(
                raw_report[start + 3].to_bytes_be()[31] as u32
                    | (raw_report[start + 3].to_bytes_be()[30] as u32) << 8
                    | (raw_report[start + 3].to_bytes_be()[29] as u32) << 16
                    | (raw_report[start + 3].to_bytes_be()[28] as u32) << 24,
            );
            w0 + (w1 << 32) + (w2 << 64) + (w3 << 96)
        };

        let limb0 = read_limb(REPORT_DATA_WORD_OFFSET);
        let limb1 = read_limb(REPORT_DATA_WORD_OFFSET + 4);
        let limb2 = read_limb(REPORT_DATA_WORD_OFFSET + 8);
        let limb3 = read_limb(REPORT_DATA_WORD_OFFSET + 12);

        assert_eq!(limb2, 0, "limb2 must be zero");
        assert_eq!(limb3, 0, "limb3 must be zero");

        let high = limb0.swap_bytes();
        let low = limb1.swap_bytes();

        let mut reconstructed = [0u8; 32];
        reconstructed[..16].copy_from_slice(&high.to_be_bytes());
        reconstructed[16..].copy_from_slice(&low.to_be_bytes());

        assert_eq!(Felt::from_bytes_be(&reconstructed), commitment);
    }

    #[test]
    fn serialized_journal_has_expected_length() {
        let felts = serialize_mock_journal(Felt::from(1u64));
        // Expected total = 306 felts (see docstring).
        assert_eq!(felts.len(), 306);
    }
}
