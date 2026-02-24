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

    // Head (10 words)
    words.append(u256_from_u128(0)); // slot 0: result = Success
    words.append(u256_from_u128(42)); // slot 1: timestamp
    words.append(u256_from_u128(1)); // slot 2: processorModel
    words.append(u256_from_u128(320)); // slot 3: rawReport offset (10*32 = 320)
    words.append(u256_from_u128(1536)); // slot 4: certs offset (320 + 1216)
    words.append(u256_from_u128(1600)); // slot 5: certSerials offset (1536 + 64)
    words.append(u256_from_u128(2)); // slot 6: trustedCertsPrefixLen
    words.append(u256_from_u128(0)); // slot 7: storageCommitment
    words.append(u256_from_u128(100)); // slot 8: forkBlockNumber
    words.append(u256_from_u128(200)); // slot 9: endBlockNumber

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
    assert(journal.raw_report.len() == ATTESTATION_REPORT_SIZE_U32.into(), 'Wrong raw report size');
    assert(journal.certs.len() == 1, 'Wrong cert count');
    assert(*journal.certs.at(0) == u256_from_u128(0x1234), 'Wrong cert value');
    assert(journal.cert_serials.len() == 1, 'Wrong serial count');
    assert(*journal.cert_serials.at(0) == 0xdead, 'Wrong serial value');
    assert(journal.storage_commitment == 0, 'Wrong storage commitment');
    assert(journal.fork_block_number == 100, 'Wrong fork block number');
    assert(journal.end_block_number == 200, 'Wrong end block number');
}
