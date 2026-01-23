use amd_tee_registry::journal_decode::decode_verifier_journal;
use amd_tee_registry::tee_types::{ATTESTATION_REPORT_SIZE_U32, VerificationResult};

fn u256_from_u128(value: u128) -> u256 {
    u256 { low: value, high: 0 }
}

#[test]
fn test_decode_verifier_journal_minimal() {
    let mut words: Array<u256> = array![];

    // ABI offset pointer (required by decode_verifier_journal)
    words.append(u256_from_u128(0x20)); // offset = 32 bytes

    // Head (7 words)
    words.append(u256_from_u128(0)); // result = Success
    words.append(u256_from_u128(42)); // timestamp
    words.append(u256_from_u128(1)); // processorModel
    words.append(u256_from_u128(224)); // rawReport offset (relative to struct start)
    words.append(u256_from_u128(1440)); // certs offset (relative to struct start)
    words.append(u256_from_u128(1504)); // certSerials offset (relative to struct start)
    words.append(u256_from_u128(2)); // trustedCertsPrefixLen

    // rawReport block
    words.append(u256_from_u128(1184)); // length in bytes
    let mut i: usize = 0;
    while i < 37 {
        words.append(u256_from_u128(0));
        i += 1;
    }

    // certs block
    words.append(u256_from_u128(1)); // length
    words.append(u256_from_u128(0x1234));

    // certSerials block
    words.append(u256_from_u128(1)); // length
    words.append(u256_from_u128(0xdead));

    let journal = decode_verifier_journal(words.span());

    assert(journal.result == VerificationResult::Success, 'Wrong result');
    assert(journal.timestamp == 42, 'Wrong timestamp');
    assert(journal.processor_model == 1, 'Wrong processor model');
    assert(journal.trusted_certs_prefix_len == 2, 'Wrong trusted prefix length');
    assert(
        journal.raw_report.len() == ATTESTATION_REPORT_SIZE_U32.into(),
        'Wrong raw report size',
    );
    assert(journal.certs.len() == 1, 'Wrong cert count');
    assert(*journal.certs.at(0) == u256_from_u128(0x1234), 'Wrong cert value');
    assert(journal.cert_serials.len() == 1, 'Wrong serial count');
    assert(*journal.cert_serials.at(0) == 0xdead, 'Wrong serial value');
}
