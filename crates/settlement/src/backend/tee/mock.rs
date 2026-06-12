//! Helpers for synthesizing fake `TEEInput.sp1_proof` payloads when the node
//! runs a mock attester.
//!
//! In mock mode the prover does **not** call AMD KDS, validate any cert chain,
//! or submit anything to the SP1 prover network. Instead it computes the v1
//! Poseidon commitment that Piltover's `validate_input` would otherwise extract
//! from a real attestation report and packages it into a Cairo-Serde-serialized
//! `amd_tee_registry::tee_types::VerifierJournal` which the paired
//! `piltover_mock_amd_tee_registry` Cairo contract trivially round-trips.
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
//! Within `raw_report`, only the 32 u32 words at the `report_data` offset
//! (u32 index 20) carry meaningful data; all other words are zero.
//!
//! ## `report_data` byte layout
//!
//! Piltover (`src/input/component.cairo`, `validate_input` for `TeeInput`)
//! decodes the 64-byte `report_data` as two big-endian 32-byte halves:
//!
//! - bytes 0..32 (u32 words [20..28)) — the v1 appchain commitment, recomputed on-chain from the
//!   `TEEInput` fields and asserted equal;
//! - bytes 32..64 (u32 words [28..36)) — the `katana_tee_config_hash`, asserted equal to
//!   `TEEInput.katana_tee_config_hash`.
//!
//! `get_u128_at` reads 4 consecutive u32 words and combines them
//! **little-endian**: `limb = w0 + w1·2^32 + w2·2^64 + w3·2^96`. Then
//! `u128_byte_reverse` swaps the byte order, converting LE → BE. Composing
//! these, each 32-byte half is exactly the felt's `to_bytes_be()` encoding,
//! packed by reading each 4-byte BE chunk as a **little-endian** `u32`. See
//! [`felt_to_report_words`] for the implementation.

use katana_primitives::Felt;

/// Number of u32 words in an AMD SEV-SNP attestation report (1184 bytes / 4).
pub const ATTESTATION_REPORT_WORDS: usize = 296;

/// u32 word offset of the `report_data` field within an attestation report
/// (byte offset 0x50 / 4).
pub const REPORT_DATA_WORD_OFFSET: usize = 20;

/// Encodes a felt's 32 big-endian bytes into 8 u32 words such that Piltover's
/// limb reconstruction (`get_u128_at` + `u128_byte_reverse`) yields the
/// original value.
///
/// Each 4-byte BE chunk of `value.to_bytes_be()` is interpreted as a
/// little-endian `u32`. See module docs for the derivation.
fn felt_to_report_words(value: Felt) -> [Felt; 8] {
    let bytes = value.to_bytes_be();
    let mut words = [Felt::ZERO; 8];
    for i in 0..8 {
        let chunk = [bytes[i * 4], bytes[i * 4 + 1], bytes[i * 4 + 2], bytes[i * 4 + 3]];
        let word = u32::from_le_bytes(chunk);
        words[i] = Felt::from(word);
    }
    words
}

/// Builds a 296-word `raw_report` whose `report_data` field encodes the v1
/// commitment in its first half and the config hash in its second half, per
/// the layout documented in [`felt_to_report_words`]. All other words are
/// zero.
pub fn build_raw_report(commitment: Felt, katana_tee_config_hash: Felt) -> Vec<Felt> {
    let mut raw_report = vec![Felt::ZERO; ATTESTATION_REPORT_WORDS];

    let commitment_words = felt_to_report_words(commitment);
    raw_report[REPORT_DATA_WORD_OFFSET..REPORT_DATA_WORD_OFFSET + 8]
        .copy_from_slice(&commitment_words);

    let config_hash_words = felt_to_report_words(katana_tee_config_hash);
    raw_report[REPORT_DATA_WORD_OFFSET + 8..REPORT_DATA_WORD_OFFSET + 16]
        .copy_from_slice(&config_hash_words);

    raw_report
}

/// Cairo-Serde-serializes a stub `VerifierJournal` whose `raw_report` field
/// encodes the given v1 Poseidon commitment and config hash in the positions
/// Piltover reads.
///
/// The output is a `Vec<Felt>` matching what
/// `Serde::<VerifierJournal>::deserialize` reconstructs in
/// `piltover_mock_amd_tee_registry::verify_sp1_proof`.
pub fn serialize_mock_journal(commitment: Felt, katana_tee_config_hash: Felt) -> Vec<Felt> {
    let raw_report = build_raw_report(commitment, katana_tee_config_hash);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_report_has_canonical_size() {
        let raw_report = build_raw_report(Felt::from(42u64), Felt::from(7u64));
        assert_eq!(raw_report.len(), ATTESTATION_REPORT_WORDS);
    }

    #[test]
    fn report_data_zero_outside_first_64_bytes() {
        let raw_report = build_raw_report(Felt::from(42u64), Felt::from(7u64));
        // Words [0..20) and [36..296) must be zero.
        for (i, word) in raw_report.iter().enumerate().take(REPORT_DATA_WORD_OFFSET) {
            assert_eq!(*word, Felt::ZERO, "word {i} should be zero");
        }
        for (i, word) in raw_report.iter().enumerate().take(ATTESTATION_REPORT_WORDS).skip(36) {
            assert_eq!(*word, Felt::ZERO, "word {i} should be zero");
        }
    }

    /// Mirror Piltover's reconstruction:
    ///   limb_i = sum(w_{4i+j} * 2^(32*j)) for j in 0..4   (little-endian u32 → u128)
    ///   value  = (u128_byte_reverse(limb_even) << 128) | u128_byte_reverse(limb_odd)
    fn reconstruct(raw_report: &[Felt], start: usize) -> Felt {
        let read_limb = |start: usize| -> u128 {
            let word = |i: usize| -> u32 {
                let b = raw_report[start + i].to_bytes_be();
                b[31] as u32 | (b[30] as u32) << 8 | (b[29] as u32) << 16 | (b[28] as u32) << 24
            };
            u128::from(word(0))
                + (u128::from(word(1)) << 32)
                + (u128::from(word(2)) << 64)
                + (u128::from(word(3)) << 96)
        };

        let high = read_limb(start).swap_bytes();
        let low = read_limb(start + 4).swap_bytes();

        let mut bytes = [0u8; 32];
        bytes[..16].copy_from_slice(&high.to_be_bytes());
        bytes[16..].copy_from_slice(&low.to_be_bytes());
        Felt::from_bytes_be(&bytes)
    }

    #[test]
    fn report_data_round_trips_commitment_and_config_hash() {
        let commitment =
            Felt::from_hex("0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap();
        let config_hash =
            Felt::from_hex("0x04edcba9876543210fedcba9876543210fedcba9876543210fedcba987654321")
                .unwrap();
        let raw_report = build_raw_report(commitment, config_hash);

        assert_eq!(reconstruct(&raw_report, REPORT_DATA_WORD_OFFSET), commitment);
        assert_eq!(reconstruct(&raw_report, REPORT_DATA_WORD_OFFSET + 8), config_hash);
    }

    #[test]
    fn serialized_journal_has_expected_length() {
        let felts = serialize_mock_journal(Felt::from(1u64), Felt::from(2u64));
        // Expected total = 306 felts (see docstring).
        assert_eq!(felts.len(), 306);
    }
}
