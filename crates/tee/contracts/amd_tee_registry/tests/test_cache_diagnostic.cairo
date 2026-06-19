// Diagnostic test for certificate cache mechanism
use amd_tee_registry::cert_cache::CertCacheComponent::{
    ICertCacheDispatcher, ICertCacheDispatcherTrait,
};
use amd_tee_registry::tee_types::ProcessorType;
use snforge_std::{ContractClassTrait, DeclareResultTrait, declare};
use starknet::ContractAddress;

// Test scenario: Simulate the E2E flow
// 1. Deploy with root cert only (no intermediate certs) - like LIVE mode first deployment
// 2. Call check_trusted_intermediate_certs with a cert chain
// 3. Verify it returns trusted_prefix_len = 1

fn deploy_with_root_only() -> ContractAddress {
    let contract = declare("AMDTEERegistry").unwrap().contract_class();

    // Root cert (ARK) hash - simulating SHA256(ARK_DER)
    let root_cert: u256 = 0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890;

    let verifier_class_hash: felt252 = 0x1;
    let sp1_program_id: u256 = 0x2;
    let max_time_diff: u64 = 3600;

    let mut calldata: Array<felt252> = array![];

    // verifier_class_hash
    calldata.append(verifier_class_hash);

    // sp1_program_id
    calldata.append(sp1_program_id.low.into());
    calldata.append(sp1_program_id.high.into());

    // max_time_diff
    calldata.append(max_time_diff.into());

    // trusted_certs array - EMPTY (simulating live mode first deployment)
    calldata.append(0); // array length = 0

    // processor_models array (1 element)
    calldata.append(1); // array length
    calldata.append(0); // ProcessorType::Milan

    // root_certs array (1 element)
    calldata.append(1); // array length
    calldata.append(root_cert.low.into());
    calldata.append(root_cert.high.into());

    // storage_commitment_proxy (0 = disabled)
    calldata.append(0);

    let (contract_address, _) = contract.deploy(@calldata).unwrap();
    contract_address
}

#[test]
fn test_cache_query_with_empty_trusted_certs() {
    let contract_address = deploy_with_root_only();
    let dispatcher = ICertCacheDispatcher { contract_address };

    // Simulate cert chain from prover: [ARK_hash, ASK_path_digest, VCEK_path_digest]
    // These are PATH digests, not individual cert hashes
    let root_cert: u256 = 0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890;
    let ask_path_digest: u256 = 0x1111111111111111111111111111111111111111111111111111111111111111;
    let vcek_path_digest: u256 = 0x2222222222222222222222222222222222222222222222222222222222222222;

    // Build the certs array as the Rust client would send
    let certs: Array<u256> = array![root_cert, ask_path_digest, vcek_path_digest];

    // Query the cache
    let processor_models: Array<ProcessorType> = array![ProcessorType::Milan];
    let report_certs: Array<Span<u256>> = array![certs.span()];

    let results = dispatcher
        .check_trusted_intermediate_certs(processor_models.span(), report_certs.span());

    // Should return 1 (only root cert is trusted)
    assert(*results.at(0) == 1, 'Expected prefix len 1');
}

#[test]
fn test_cache_query_with_initialized_ask() {
    let contract = declare("AMDTEERegistry").unwrap().contract_class();

    // Root cert (ARK) hash
    let root_cert: u256 = 0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890;
    // ASK path digest - this is what gets stored as trusted intermediate
    let ask_path_digest: u256 = 0x1111111111111111111111111111111111111111111111111111111111111111;

    let verifier_class_hash: felt252 = 0x1;
    let sp1_program_id: u256 = 0x2;
    let max_time_diff: u64 = 3600;

    let mut calldata: Array<felt252> = array![];
    calldata.append(verifier_class_hash);
    calldata.append(sp1_program_id.low.into());
    calldata.append(sp1_program_id.high.into());
    calldata.append(max_time_diff.into());

    // trusted_certs array - includes ASK path digest (simulating fixture mode)
    calldata.append(1); // array length = 1
    calldata.append(ask_path_digest.low.into());
    calldata.append(ask_path_digest.high.into());

    // processor_models array
    calldata.append(1);
    calldata.append(0); // Milan

    // root_certs array
    calldata.append(1);
    calldata.append(root_cert.low.into());
    calldata.append(root_cert.high.into());

    // storage_commitment_proxy (0 = disabled)
    calldata.append(0);

    let (contract_address, _) = contract.deploy(@calldata).unwrap();
    let dispatcher = ICertCacheDispatcher { contract_address };

    // Query with same cert chain
    let vcek_path_digest: u256 = 0x2222222222222222222222222222222222222222222222222222222222222222;
    let certs: Array<u256> = array![root_cert, ask_path_digest, vcek_path_digest];
    let processor_models: Array<ProcessorType> = array![ProcessorType::Milan];
    let report_certs: Array<Span<u256>> = array![certs.span()];

    let results = dispatcher
        .check_trusted_intermediate_certs(processor_models.span(), report_certs.span());

    // Should return 2 (root + ASK are trusted)
    assert(*results.at(0) == 2, 'Expected prefix len 2');
}

#[test]
#[should_panic(expected: "First certificate must be root certificate")]
fn test_root_cert_mismatch_panics() {
    let contract_address = deploy_with_root_only();
    let dispatcher = ICertCacheDispatcher { contract_address };

    // Wrong root cert
    let wrong_root: u256 = 0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef;
    let ask_path_digest: u256 = 0x1111111111111111111111111111111111111111111111111111111111111111;

    let certs: Array<u256> = array![wrong_root, ask_path_digest];
    let processor_models: Array<ProcessorType> = array![ProcessorType::Milan];
    let report_certs: Array<Span<u256>> = array![certs.span()];

    // This should panic with "First certificate must be root certificate"
    let _results = dispatcher
        .check_trusted_intermediate_certs(processor_models.span(), report_certs.span());
}

#[test]
fn test_verify_root_cert_storage() {
    let contract_address = deploy_with_root_only();
    let dispatcher = ICertCacheDispatcher { contract_address };

    let expected_root: u256 = 0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890;

    // Verify root cert is stored correctly
    let stored_root = dispatcher.get_root_cert(ProcessorType::Milan);
    assert(stored_root == expected_root, 'Root cert mismatch');

    // Genoa should be zero (not set)
    let genoa_root = dispatcher.get_root_cert(ProcessorType::Genoa);
    assert(genoa_root == 0, 'Genoa should be zero');
}
