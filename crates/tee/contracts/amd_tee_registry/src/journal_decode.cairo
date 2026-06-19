use crate::tee_types::{ATTESTATION_REPORT_SIZE_U32, VerificationResult, VerifierJournal};

const POW_32_U128: u128 = 0x100000000;
const POW_64_U128: u128 = 0x10000000000000000;
const POW_96_U128: u128 = 0x1000000000000000000000000;
const POW_128_FELT: felt252 = 0x100000000000000000000000000000000;

/// Convert SP1 public inputs from `Span<u256>` to `Array<u32>` in big-endian word order.
pub fn u256_span_to_u32_array(words: Span<u256>) -> Array<u32> {
    let mut output: Array<u32> = array![];
    let mut i: usize = 0;
    while i < words.len() {
        let word = *words.at(i);
        let (w7, w6, w5, w4, w3, w2, w1, w0) = split_u256_to_u32_be(word);
        output.append(w7);
        output.append(w6);
        output.append(w5);
        output.append(w4);
        output.append(w3);
        output.append(w2);
        output.append(w1);
        output.append(w0);
        i += 1;
    }
    output
}

/// Decode the Solidity-ABI `VerifierJournal` from SP1 public inputs.
///
/// Note: The journal is ABI-encoded as a struct with dynamic fields, which means
/// it starts with a 32-byte offset pointer (0x20) before the actual data.
/// We skip this offset and decode starting from the actual struct data.
pub fn decode_verifier_journal(public_inputs: Span<u256>) -> VerifierJournal {
    let words_u32 = u256_span_to_u32_array(public_inputs);
    // Skip the first 8 u32 words (32 bytes) which is the ABI offset pointer
    let data_start = 8_usize;
    decode_verifier_journal_from_u32(
        words_u32.span().slice(data_start, words_u32.len() - data_start),
    )
}

fn decode_verifier_journal_from_u32(words: Span<u32>) -> VerifierJournal {
    assert(words.len() % 8 == 0, 'Invalid ABI length');
    assert(words.len() >= 10 * 8, 'ABI too short');

    let result_word = read_word_u32(words, 0);
    let timestamp_word = read_word_u32(words, 1);
    let processor_word = read_word_u32(words, 2);
    let raw_report_offset_word = read_word_u32(words, 3);
    let certs_offset_word = read_word_u32(words, 4);
    let cert_serials_offset_word = read_word_u32(words, 5);
    let trusted_prefix_word = read_word_u32(words, 6);
    let storage_commitment = word_to_u256(read_word_u32(words, 7));

    let result = verification_result_from_u8(word_to_u8(result_word));
    let timestamp = word_to_u64(timestamp_word);
    let processor_model = word_to_u8(processor_word);
    let trusted_certs_prefix_len = word_to_u8(trusted_prefix_word);

    let raw_report_offset_bytes = word_to_u64(raw_report_offset_word);
    let certs_offset_bytes = word_to_u64(certs_offset_word);
    let cert_serials_offset_bytes = word_to_u64(cert_serials_offset_word);

    let raw_report_offset_words: usize = (raw_report_offset_bytes / 32).try_into().unwrap();
    let certs_offset_words: usize = (certs_offset_bytes / 32).try_into().unwrap();
    let cert_serials_offset_words: usize = (cert_serials_offset_bytes / 32).try_into().unwrap();

    let raw_report_len_bytes = word_to_u64(read_word_u32(words, raw_report_offset_words));
    assert(raw_report_len_bytes % 4 == 0, 'Raw report length misaligned');
    let raw_report_len_u32: usize = (raw_report_len_bytes / 4).try_into().unwrap();
    assert(raw_report_len_u32 == ATTESTATION_REPORT_SIZE_U32.into(), 'Unexpected report size');

    let raw_report_start_u32 = (raw_report_offset_words + 1) * 8;
    let mut raw_report_words: Array<u32> = array![];
    let mut i: usize = 0;
    while i < raw_report_len_u32 {
        let word_be = *words.at(raw_report_start_u32 + i);
        raw_report_words.append(swap_endian_u32(word_be));
        i += 1;
    }

    let certs_len = word_to_u64(read_word_u32(words, certs_offset_words));
    let certs_len_usize: usize = certs_len.try_into().unwrap();
    let mut certs: Array<u256> = array![];
    let mut j: usize = 0;
    while j < certs_len_usize {
        let word = read_word_u32(words, certs_offset_words + 1 + j);
        certs.append(word_to_u256(word));
        j += 1;
    }

    let cert_serials_len = word_to_u64(read_word_u32(words, cert_serials_offset_words));
    let cert_serials_len_usize: usize = cert_serials_len.try_into().unwrap();
    let mut cert_serials: Array<felt252> = array![];
    let mut k: usize = 0;
    while k < cert_serials_len_usize {
        let word = read_word_u32(words, cert_serials_offset_words + 1 + k);
        cert_serials.append(u256_to_felt(word_to_u256(word)));
        k += 1;
    }

    let fork_block_number = word_to_u64(read_word_u32(words, 8));
    let end_block_number = word_to_u64(read_word_u32(words, 9));

    VerifierJournal {
        result,
        timestamp,
        processor_model,
        raw_report: raw_report_words.span(),
        certs,
        cert_serials,
        trusted_certs_prefix_len,
        storage_commitment: u256_to_felt(storage_commitment),
        fork_block_number,
        end_block_number,
    }
}

fn split_u256_to_u32_be(value: u256) -> (u32, u32, u32, u32, u32, u32, u32, u32) {
    let (h3, h2, h1, h0) = split_u128_to_u32_be(value.high);
    let (l3, l2, l1, l0) = split_u128_to_u32_be(value.low);
    (h3, h2, h1, h0, l3, l2, l1, l0)
}

fn split_u128_to_u32_be(value: u128) -> (u32, u32, u32, u32) {
    let w0: u128 = value % POW_32_U128;
    let w1: u128 = (value / POW_32_U128) % POW_32_U128;
    let w2: u128 = (value / POW_64_U128) % POW_32_U128;
    let w3: u128 = value / POW_96_U128;
    (w3.try_into().unwrap(), w2.try_into().unwrap(), w1.try_into().unwrap(), w0.try_into().unwrap())
}

fn u128_from_u32_be(w3: u32, w2: u32, w1: u32, w0: u32) -> u128 {
    let w0: u128 = w0.into();
    let w1: u128 = w1.into();
    let w2: u128 = w2.into();
    let w3: u128 = w3.into();
    w0 + (w1 * POW_32_U128) + (w2 * POW_64_U128) + (w3 * POW_96_U128)
}

fn read_word_u32(words: Span<u32>, word_index: usize) -> (u32, u32, u32, u32, u32, u32, u32, u32) {
    let start = word_index * 8;
    (
        *words.at(start),
        *words.at(start + 1),
        *words.at(start + 2),
        *words.at(start + 3),
        *words.at(start + 4),
        *words.at(start + 5),
        *words.at(start + 6),
        *words.at(start + 7),
    )
}

fn word_to_u256(word: (u32, u32, u32, u32, u32, u32, u32, u32)) -> u256 {
    let (w7, w6, w5, w4, w3, w2, w1, w0) = word;
    let high = u128_from_u32_be(w7, w6, w5, w4);
    let low = u128_from_u32_be(w3, w2, w1, w0);
    u256 { low, high }
}

fn word_to_u64(word: (u32, u32, u32, u32, u32, u32, u32, u32)) -> u64 {
    let (_, _, _, _, _, _, w1, w0) = word;
    let high: u64 = w1.into();
    let low: u64 = w0.into();
    high * 0x100000000 + low
}

fn word_to_u8(word: (u32, u32, u32, u32, u32, u32, u32, u32)) -> u8 {
    let (_, _, _, _, _, _, _, w0) = word;
    let byte = w0 % 256;
    byte.try_into().unwrap()
}

fn swap_endian_u32(word: u32) -> u32 {
    let b0 = word / 16777216;
    let b1 = (word / 65536) % 256;
    let b2 = (word / 256) % 256;
    let b3 = word % 256;
    (b3 * 16777216) + (b2 * 65536) + (b1 * 256) + b0
}

fn u256_to_felt(value: u256) -> felt252 {
    let low: felt252 = value.low.into();
    let high: felt252 = value.high.into();
    low + (high * POW_128_FELT)
}

fn verification_result_from_u8(value: u8) -> VerificationResult {
    match value {
        0 => VerificationResult::Success,
        1 => VerificationResult::RootCertNotTrusted,
        2 => VerificationResult::IntermediateCertsNotTrusted,
        3 => VerificationResult::InvalidTimestamp,
        _ => panic!("Unknown verification result"),
    }
}
